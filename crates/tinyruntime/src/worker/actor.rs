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

use super::native::{self, Preparation, Process};
use super::{Command, State, outcome};

const REPLAY_WINDOW: u64 = 8;
const BACKOFF: Duration = Duration::from_secs(300);

struct Actor {
    plan: Arc<WorkerPlan>,
    root: PathBuf,
    process: Option<Process>,
    preparation: Option<Preparation>,
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
    mut commands: mpsc::Receiver<Command>,
    stop: watch::Receiver<bool>,
    state: watch::Sender<State>,
) {
    let mut actor = Actor {
        plan,
        root,
        process: None,
        preparation: None,
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
            match tokio::fs::remove_dir_all(&actor.root).await {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(_) => {}
            }
        }
        actor.state.send_modify(|s| s.cleanup_failed = true);
        if actor.stop.changed().await.is_err() {
            // Module drop has no public retry caller: continue retaining/reaping.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    actor.state.send_modify(|s| {
        s.closed = true;
        s.status.phase = WorkerPhase::Closed;
        s.status.server.running = false;
    });
    // Dropping queued reply senders communicates closure without repeating work.
}

impl Actor {
    async fn serve(&mut self, commands: &mut mpsc::Receiver<Command>) {
        let prepared = self.prepare().await;
        self.state.send_modify(|s| {
            s.prepared = Some(prepared.clone());
            s.status.phase = if prepared.kind == WorkerOutcomeKind::Complete {
                WorkerPhase::Prepared
            } else {
                WorkerPhase::Failed
            };
        });
        loop {
            let command = tokio::select! {
                biased;
                () = native::canceled(&mut self.stop) => break,
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
                    let _ = reply.send(self.request(request, deadline).await);
                }
            }
        }
    }
    async fn prepare(&mut self) -> WorkerOutcome {
        let deadline = Instant::now() + Duration::from_millis(self.plan.startup_timeout_ms);
        if tokio::fs::create_dir_all(&self.root).await.is_err()
            || tokio::fs::write(self.root.join("worker-script"), &self.plan.source)
                .await
                .is_err()
        {
            return outcome(WorkerOutcomeKind::Failed, Some("install_failed"));
        }
        for step in &self.plan.preparation {
            match Preparation::spawn(step, &self.root) {
                Ok(process) => self.preparation = Some(process),
                Err(reason) => return outcome(WorkerOutcomeKind::Failed, Some(reason)),
            }
            let Some(preparation) = self.preparation.as_mut() else {
                return outcome(WorkerOutcomeKind::Failed, Some("prepare_closed"));
            };
            let result = preparation.execute(&mut self.stop, deadline).await;
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

    async fn cleanup(&mut self) -> Result<(), &'static str> {
        if let Some(process) = &mut self.preparation {
            process.cleanup().await?;
        }
        self.preparation = None;
        if let Some(process) = &mut self.process {
            process.cleanup().await?;
        }
        self.process = None;
        self.ready_backends.clear();
        self.state.send_modify(|s| {
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
        if self.process.is_some() {
            return outcome(WorkerOutcomeKind::Complete, None);
        }
        if self.retry_after.is_some_and(|after| Instant::now() < after) {
            return outcome(WorkerOutcomeKind::Failed, Some("startup_backoff"));
        }
        self.state
            .send_modify(|s| s.status.phase = WorkerPhase::Starting);
        let result = match Process::spawn(
            &self.plan.command,
            &self.root,
            &self.root.join("worker-script"),
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
        let result = self.dispatch(&request, deadline).await;
        self.highest = self.highest.max(request.operation);
        self.replies
            .retain(|id, _| *id > self.highest.saturating_sub(REPLAY_WINDOW));
        self.replies
            .insert(request.operation, (request, result.clone()));
        result
    }

    async fn dispatch(&mut self, request: &WorkerRequest, deadline: Instant) -> WorkerOutcome {
        let wire = ServerRequest {
            id: request.operation.to_string(),
            method: request.method.clone(),
            params: request.params.clone(),
        };
        if Instant::now() >= deadline {
            return outcome(WorkerOutcomeKind::Failed, Some("request_timeout"));
        }
        let mut reason = "worker_closed";
        for attempt in 0..2 {
            let Ok(started) = tokio::time::timeout_at(deadline, self.start()).await else {
                if self.cleanup().await.is_err() {
                    return outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"));
                }
                return outcome(WorkerOutcomeKind::Failed, Some("request_timeout"));
            };
            if started.kind != WorkerOutcomeKind::Complete {
                return started;
            }
            let Some(process) = &mut self.process else {
                return outcome(WorkerOutcomeKind::Failed, Some("worker_closed"));
            };
            let result = tokio::select! {
                biased;
                () = native::canceled(&mut self.stop) => Err("canceled"),
                result = tokio::time::timeout_at(deadline, process.request(&wire)) => result.unwrap_or(Err("request_timeout")),
            };
            match result {
                Ok(response) if response.ok || attempt == 1 => {
                    if response.ok {
                        self.last_used = Instant::now();
                    }
                    return WorkerOutcome {
                        kind: WorkerOutcomeKind::Response,
                        response: Some(response),
                        reason: None,
                    };
                }
                Ok(_) => reason = "remote_failed",
                Err(error) => reason = error,
            }
            if self.cleanup().await.is_err() {
                return outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"));
            }
            if *self.stop.borrow() || Instant::now() >= deadline {
                break;
            }
        }
        outcome(WorkerOutcomeKind::Failed, Some(reason))
    }
}
