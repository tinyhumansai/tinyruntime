//! Detached per-resource ownership, bounded replay, startup backoff and idle rebuild.
use futures_util::FutureExt;
use std::collections::BTreeMap;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tinyruntime_bus::worker::{
    BackendStatus, ServerRequest, ServerStatus, WorkerOutcome, WorkerOutcomeKind, WorkerPhase,
    WorkerPlan, WorkerRequest,
};
use tokio::sync::{mpsc, watch};
use tokio::time::Instant;

use super::cache::Session;
use super::native::{self, Preparation, Process};
use super::{Cached, Command, State, outcome};

const REPLAY_WINDOW: u64 = 8;
const BACKOFF: Duration = Duration::from_secs(300);

struct Actor {
    plan: Arc<WorkerPlan>,
    root: PathBuf,
    directory: Option<cap_std::fs::Dir>,
    process: Option<Process>,
    preparation: Option<Preparation>,
    cached: Option<Cached>,
    cache: Option<Session>,
    ready_backends: Vec<String>,
    last_used: Instant,
    retry_after: Option<Instant>,
    stop: watch::Receiver<bool>,
    state: watch::Sender<State>,
    replies: BTreeMap<u64, (WorkerRequest, WorkerOutcome)>,
    highest: u64,
}

pub(super) async fn run(
    plan: Arc<WorkerPlan>,
    root: PathBuf,
    cached: Option<Cached>,
    mut commands: mpsc::Receiver<Command>,
    stop: watch::Receiver<bool>,
    state: watch::Sender<State>,
) {
    let mut actor = Actor {
        plan,
        root,
        directory: None,
        process: None,
        preparation: None,
        cached,
        cache: None,
        ready_backends: Vec::new(),
        last_used: Instant::now(),
        retry_after: None,
        stop,
        state,
        replies: BTreeMap::new(),
        highest: 0,
    };
    let _ = AssertUnwindSafe(actor.serve(&mut commands))
        .catch_unwind()
        .await;
    loop {
        actor.stop.borrow_and_update();
        if actor.cleanup().await.is_ok() {
            match actor.remove_directory() {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => {}
            }
        }
        actor.state.send_modify(|s| s.cleanup_failed = true);
        // Retry retained cleanup even when Stop has already been observed.
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    actor.state.send_modify(|s| {
        s.closed = true;
        s.status.phase = WorkerPhase::Closed;
        s.status.server.running = false;
    });
    // Dropping queued reply senders communicates closure without repeating work.
}

impl Actor {
    // Cleanup follows the retained capability even after its ambient path changes.
    // A clone keeps ownership available for retries if removal fails.
    fn remove_directory(&self) -> std::io::Result<()> {
        match &self.directory {
            Some(directory) => directory.try_clone()?.remove_open_dir_all(),
            // Preparation never acquired this directory, so nothing is ours to remove.
            None => Ok(()),
        }
    }

    async fn serve(&mut self, commands: &mut mpsc::Receiver<Command>) {
        let prepared = self.prepare().await;
        self.state.send_modify(|s| {
            s.prepared = Some(prepared.clone());
            s.status.phase = if prepared.kind == WorkerOutcomeKind::Complete {
                s.status.server = ServerStatus {
                    enabled: true,
                    running: false,
                    backends: self
                        .plan
                        .backends
                        .iter()
                        .map(|id| BackendStatus {
                            id: id.clone(),
                            enabled: true,
                            ready: false,
                            message: None,
                        })
                        .collect(),
                    message: None,
                };
                WorkerPhase::Prepared
            } else {
                WorkerPhase::Failed
            };
        });
        let mut supervision = tokio::time::interval(Duration::from_millis(100));
        supervision.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let command = tokio::select! {
                biased;
                () = native::canceled(&mut self.stop) => break,
                _ = supervision.tick() => {
                    let unusable = self.process.as_mut().is_some_and(|process| !process.is_alive());
                    let failed_preparation = prepared.kind != WorkerOutcomeKind::Complete
                        && (self.preparation.is_some() || self.cache.is_some());
                    if unusable || failed_preparation {
                        let _ = self.cleanup().await;
                    }
                    continue;
                },
                command = commands.recv() => match command { Some(command) => command, None => break },
            };
            match command {
                #[cfg(test)]
                Command::FailReap(reply) => {
                    if let Some(process) = &mut self.process {
                        process.reap_failures += 1;
                    }
                    let _ = reply.send(());
                }
                #[cfg(test)]
                Command::GateReap(entered, release, reply) => {
                    if let Some(process) = &mut self.process {
                        process.reap_gate = Some((entered, release));
                    }
                    let _ = reply.send(());
                }
                Command::Start(reply) => {
                    let _ = reply.send(self.start().await);
                }
                Command::Request(request, deadline, reply) => {
                    // Caller cancellation before dequeue owns no native side effects.
                    if reply.is_closed() {
                        continue;
                    }
                    let _ = reply.send(self.request(request, deadline).await);
                }
            }
        }
    }
    async fn prepare_cache(&mut self) -> Result<(), &'static str> {
        let Some(cached) = self.cached.clone() else {
            return Ok(());
        };
        let deadline = Instant::now() + Duration::from_millis(cached.recipe.timeout_ms);
        self.cache =
            Some(Session::acquire(&cached.root, &cached.recipe, &mut self.stop, deadline).await?);
        let result = self.provision_cache(&cached, deadline).await;
        // Native failures keep the preparation + session owned for public Stop retry.
        if let Some(preparation) = &mut self.preparation {
            preparation.cleanup().await?;
        }
        self.preparation = None;
        if let Some(cache) = &mut self.cache {
            cache.cleanup()?;
        }
        self.cache = None;
        result
    }

    async fn provision_cache(
        &mut self,
        cached: &Cached,
        deadline: Instant,
    ) -> Result<(), &'static str> {
        let session = self.cache.as_mut().ok_or("cache_session_missing")?;
        if session.legacy {
            return session.adopt_legacy(&cached.recipe);
        }
        if session.cached {
            return Ok(());
        }
        session.install(&cached.recipe)?;
        for step in &cached.recipe.steps {
            if *self.stop.borrow() {
                return Err("canceled");
            }
            if Instant::now() >= deadline {
                return Err("cache_timeout");
            }
            self.cache
                .as_ref()
                .ok_or("cache_session_missing")?
                .check_binding()?;
            self.preparation = Some(Preparation::spawn(step, &cached.root)?);
            let preparation = self.preparation.as_mut().ok_or("prepare_closed")?;
            let step_deadline = native::command_deadline(step, deadline);
            let result = preparation.execute(&mut self.stop, step_deadline).await;
            preparation.cleanup().await?;
            self.preparation = None;
            result?;
        }
        if *self.stop.borrow() {
            return Err("canceled");
        }
        if Instant::now() >= deadline {
            return Err("cache_timeout");
        }
        self.cache
            .as_ref()
            .ok_or("cache_session_missing")?
            .publish(&cached.recipe)
    }

    async fn prepare(&mut self) -> WorkerOutcome {
        if let Err(reason) = self.prepare_cache().await {
            return outcome(WorkerOutcomeKind::Failed, Some(reason));
        }
        let deadline = Instant::now() + Duration::from_millis(self.plan.startup_timeout_ms);
        if self.install_script().await.is_err() {
            return outcome(WorkerOutcomeKind::Failed, Some("install_failed"));
        }
        for step in &self.plan.preparation {
            match Preparation::spawn(step, &self.execution_root()) {
                Ok(process) => self.preparation = Some(process),
                Err(reason) => return outcome(WorkerOutcomeKind::Failed, Some(reason)),
            }
            let Some(preparation) = self.preparation.as_mut() else {
                return outcome(WorkerOutcomeKind::Failed, Some("prepare_closed"));
            };
            let step_deadline = native::command_deadline(step, deadline);
            let result = preparation.execute(&mut self.stop, step_deadline).await;
            if preparation.cleanup().await.is_err() {
                return outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"));
            }
            self.preparation = None;
            if let Err(reason) = result {
                return outcome(WorkerOutcomeKind::Failed, Some(reason));
            }
        }
        if *self.stop.borrow() {
            return outcome(WorkerOutcomeKind::Closed, Some("canceled"));
        }
        outcome(WorkerOutcomeKind::Complete, None)
    }

    async fn install_script(&mut self) -> std::io::Result<()> {
        use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
        use tokio::io::AsyncWriteExt;
        let directory =
            super::cache::open_worker_directory(&self.root).map_err(std::io::Error::other)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            directory
                .open(".")?
                .into_std()
                .set_permissions(std::fs::Permissions::from_mode(0o700))?;
        }
        self.directory = Some(directory.try_clone()?);
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .write(true)
            .create(true)
            .follow(FollowSymlinks::No)
            .nonblock(true);
        #[cfg(unix)]
        {
            use cap_std::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = directory.open_with("worker-script", &options)?;
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(std::io::ErrorKind::InvalidInput.into());
        }
        let mut file = tokio::fs::File::from_std(file.into_std());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))
                .await?;
        }
        file.set_len(0).await?;
        file.write_all(self.plan.source.as_bytes()).await
    }

    // The child changes directory before exec while this descriptor is still open.
    // Its relative script argument then resolves against the pinned working directory.
    fn execution_root(&self) -> PathBuf {
        #[cfg(unix)]
        if let Some(directory) = &self.directory {
            use std::os::fd::AsRawFd;
            #[cfg(target_os = "linux")]
            let prefix = "/proc/self/fd";
            #[cfg(not(target_os = "linux"))]
            let prefix = "/dev/fd";
            return PathBuf::from(prefix).join(directory.as_raw_fd().to_string());
        }
        self.root.clone()
    }

    async fn cleanup(&mut self) -> Result<(), &'static str> {
        self.state.send_modify(|s| {
            s.status.phase = if s
                .prepared
                .as_ref()
                .is_some_and(|result| result.kind == WorkerOutcomeKind::Complete)
            {
                WorkerPhase::Prepared
            } else {
                WorkerPhase::Failed
            };
            s.status.server.running = false;
            for backend in &mut s.status.server.backends {
                backend.ready = false;
            }
        });
        if let Some(process) = &mut self.preparation {
            process.cleanup().await?;
        }
        self.preparation = None;
        if let Some(cache) = &mut self.cache {
            cache.cleanup()?;
        }
        self.cache = None;
        if let Some(process) = &mut self.process {
            process.cleanup().await?;
        }
        self.process = None;
        self.ready_backends.clear();
        self.state.send_modify(|s| {
            s.status.phase = if s
                .prepared
                .as_ref()
                .is_some_and(|result| result.kind == WorkerOutcomeKind::Complete)
            {
                WorkerPhase::Prepared
            } else {
                WorkerPhase::Failed
            };
            s.status.server.running = false;
            for backend in &mut s.status.server.backends {
                backend.ready = false;
            }
        });
        Ok(())
    }

    async fn start(&mut self) -> WorkerOutcome {
        if *self.stop.borrow() {
            return outcome(WorkerOutcomeKind::Closed, None);
        }
        if self
            .state
            .borrow()
            .prepared
            .as_ref()
            .is_none_or(|p| p.kind != WorkerOutcomeKind::Complete)
        {
            return outcome(WorkerOutcomeKind::Failed, Some("prepare_failed"));
        }
        let idle = self.plan.idle_timeout_ms > 0
            && self
                .plan
                .idle_backend
                .as_ref()
                .is_some_and(|id| self.plan.backends.contains(id))
            && self.last_used.elapsed() >= Duration::from_millis(self.plan.idle_timeout_ms);
        if idle && self.cleanup().await.is_err() {
            return outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"));
        }
        if let Some(process) = &mut self.process {
            if self.state.borrow().status.server.running && process.is_alive() {
                return outcome(WorkerOutcomeKind::Complete, None);
            }
            if self.cleanup().await.is_err() {
                return outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"));
            }
        }
        if self.retry_after.is_some_and(|after| Instant::now() < after) {
            return outcome(WorkerOutcomeKind::Failed, Some("startup_backoff"));
        }
        self.state
            .send_modify(|s| s.status.phase = WorkerPhase::Starting);
        let result = match Process::spawn(
            &self.plan.command,
            &self.execution_root(),
            std::path::Path::new("./worker-script"),
        ) {
            Ok(process) => {
                self.process = Some(process);
                self.handshake().await
            }
            Err(reason) => Err(reason),
        };
        match result {
            Ok(backends) if !*self.stop.borrow() => {
                self.ready_backends = backends;
                self.last_used = Instant::now();
                self.retry_after = None;
                self.state.send_modify(|s| {
                    s.status.phase = WorkerPhase::Running;
                    s.status.server = ServerStatus {
                        enabled: true,
                        running: true,
                        backends: self
                            .plan
                            .backends
                            .iter()
                            .map(|id| BackendStatus {
                                id: id.clone(),
                                enabled: true,
                                ready: self.ready_backends.contains(id),
                                message: None,
                            })
                            .collect(),
                        message: None,
                    };
                });
                outcome(WorkerOutcomeKind::Complete, None)
            }
            result => {
                if self.cleanup().await.is_err() {
                    return outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"));
                }
                self.retry_after = Some(Instant::now() + BACKOFF);
                self.state
                    .send_modify(|s| s.status.phase = WorkerPhase::Failed);
                outcome(
                    WorkerOutcomeKind::Failed,
                    Some(result.err().unwrap_or("canceled")),
                )
            }
        }
    }

    async fn handshake(&mut self) -> Result<Vec<String>, &'static str> {
        let Some(process) = &mut self.process else {
            return Err("worker_closed");
        };
        tokio::select! {
            biased;
            () = native::canceled(&mut self.stop) => Err("canceled"),
            ready = tokio::time::timeout(Duration::from_millis(self.plan.startup_timeout_ms), process.handshake()) => ready.map_err(|_| "handshake_timeout")?.map(|r| r.backends),
        }
    }

    async fn request(&mut self, request: WorkerRequest, deadline: Instant) -> WorkerOutcome {
        if Instant::now() >= deadline {
            return outcome(WorkerOutcomeKind::Failed, Some("request_timeout"));
        }
        if let Some((previous, result)) = self.replies.get(&request.operation) {
            return if previous.method == request.method && previous.params == request.params {
                result.clone()
            } else {
                outcome(WorkerOutcomeKind::Invalid, Some("operation_conflict"))
            };
        }
        if request.operation <= self.highest.saturating_sub(REPLAY_WINDOW) {
            return outcome(WorkerOutcomeKind::Expired, None);
        }
        let (result, attempted) = self.dispatch(&request, deadline).await;
        if attempted {
            self.highest = self.highest.max(request.operation);
            self.replies
                .retain(|id, _| *id > self.highest.saturating_sub(REPLAY_WINDOW));
            self.replies
                .insert(request.operation, (request, result.clone()));
        }
        result
    }

    async fn dispatch(
        &mut self,
        request: &WorkerRequest,
        deadline: Instant,
    ) -> (WorkerOutcome, bool) {
        let mut attempted = false;
        let wire = ServerRequest {
            id: request.operation.to_string(),
            method: request.method.clone(),
            params: request.params.clone(),
        };
        if Instant::now() >= deadline {
            return (
                outcome(WorkerOutcomeKind::Failed, Some("request_timeout")),
                attempted,
            );
        }
        let mut reason = "worker_closed";
        for attempt in 0..2 {
            let Ok(started) = tokio::time::timeout_at(deadline, self.start()).await else {
                if self.cleanup().await.is_err() {
                    return (
                        outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed")),
                        attempted,
                    );
                }
                return (
                    outcome(WorkerOutcomeKind::Failed, Some("request_timeout")),
                    attempted,
                );
            };
            if started.kind != WorkerOutcomeKind::Complete {
                return (started, attempted);
            }
            let Some(process) = &mut self.process else {
                return (
                    outcome(WorkerOutcomeKind::Failed, Some("worker_closed")),
                    attempted,
                );
            };
            let result = tokio::select! {
                biased;
                () = native::canceled(&mut self.stop) => Err("canceled"),
                result = tokio::time::timeout_at(deadline, async {
                    attempted = true;
                    process.request(&wire).await
                }) => result.unwrap_or(Err("request_timeout")),
            };
            match result {
                Ok(response) if response.ok || attempt == 1 => {
                    if response.ok {
                        self.last_used = Instant::now();
                    }
                    return (
                        WorkerOutcome {
                            kind: WorkerOutcomeKind::Response,
                            response: Some(response),
                            reason: None,
                        },
                        attempted,
                    );
                }
                Ok(_) => reason = "remote_failed",
                Err(error) => reason = error,
            }
            if self.cleanup().await.is_err() {
                return (
                    outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed")),
                    attempted,
                );
            }
            if *self.stop.borrow() || Instant::now() >= deadline {
                break;
            }
        }
        (outcome(WorkerOutcomeKind::Failed, Some(reason)), attempted)
    }
}
