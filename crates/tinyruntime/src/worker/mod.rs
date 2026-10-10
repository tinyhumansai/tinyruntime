//! Module-owned bounded persistent JSONL workers.
//!
//! A known reservation precedes native side effects. Each prepared resource has
//! one detached supervisor; callers only own replies. Stop/shutdown wait for the
//! supervisor's native and pipe cleanup before acknowledging completion.

mod actor;
mod native;
mod validate;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub use tinyruntime_bus::worker::{
    ServerStatus, WorkerHandle, WorkerOutcome, WorkerOutcomeKind, WorkerPhase, WorkerPlan,
    WorkerPrepare, WorkerRequest, WorkerStatus,
};
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tokio::time::Instant;

const CAPACITY: usize = 16;
const QUEUE: usize = 8;
const RESERVATION_LIFETIME: Duration = Duration::from_secs(30);

fn outcome(kind: WorkerOutcomeKind, reason: Option<&str>) -> WorkerOutcome {
    WorkerOutcome {
        kind,
        response: None,
        reason: reason.map(str::to_owned),
    }
}

#[derive(Debug, Clone)]
struct State {
    status: WorkerStatus,
    prepared: Option<WorkerOutcome>,
    closed: bool,
    cleanup_failed: bool,
}

impl State {
    fn new(phase: WorkerPhase) -> Self {
        Self {
            status: WorkerStatus {
                phase,
                server: ServerStatus::disabled("worker_not_started"),
            },
            prepared: None,
            closed: false,
            cleanup_failed: false,
        }
    }
}

#[derive(Debug)]
enum Command {
    #[cfg(test)]
    FailReap(oneshot::Sender<()>),
    #[cfg(test)]
    GateReap(
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Notify>,
        oneshot::Sender<()>,
    ),
    Start(oneshot::Sender<WorkerOutcome>),
    Request(WorkerRequest, Instant, oneshot::Sender<WorkerOutcome>),
}

#[derive(Debug)]
struct Slot {
    reserved_at: Instant,
    plan: Option<Arc<WorkerPlan>>,
    commands: Option<mpsc::Sender<Command>>,
    stop: watch::Sender<bool>,
    state: watch::Sender<State>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.stop.send_replace(true);
    }
}

#[derive(Debug)]
struct Book {
    nonce: String,
    issued: u64,
    terminal: bool,
    slots: HashMap<WorkerHandle, Slot>,
}

/// One process-cached manager; admission counts live slots, not lifetime IDs.
#[derive(Debug, Clone)]
pub struct WorkerManager {
    book: Arc<Mutex<Book>>,
    root: PathBuf,
}

impl WorkerManager {
    /// Create an empty manager; native work begins only after a known reservation.
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            book: Arc::new(Mutex::new(Book {
                nonce: uuid::Uuid::new_v4().to_string(),
                issued: 0,
                terminal: false,
                slots: HashMap::new(),
            })),
        }
    }

    /// Reserve a handle without claiming an interpreter/device or spawning.
    ///
    /// # Errors
    /// Returns a bounded classification when shutdown or live capacity prevents admission.
    pub async fn reserve(&self) -> Result<WorkerHandle, WorkerOutcome> {
        let mut book = self.book.lock().await;
        if book.terminal {
            return Err(outcome(WorkerOutcomeKind::Closed, Some("manager_shutdown")));
        }
        book.slots.retain(|_, slot| {
            !slot.state.borrow().closed
                && (slot.plan.is_some() || slot.reserved_at.elapsed() < RESERVATION_LIFETIME)
        });
        if book.slots.len() >= CAPACITY {
            return Err(outcome(WorkerOutcomeKind::Busy, Some("worker_capacity")));
        }
        book.issued = book
            .issued
            .checked_add(1)
            .ok_or_else(|| outcome(WorkerOutcomeKind::Failed, Some("identity_exhausted")))?;
        let handle = WorkerHandle(format!("{}:{}", book.nonce, book.issued));
        let (stop, _) = watch::channel(false);
        let (state, _) = watch::channel(State::new(WorkerPhase::Reserved));
        book.slots.insert(
            handle.clone(),
            Slot {
                reserved_at: Instant::now(),
                plan: None,
                commands: None,
                stop,
                state,
            },
        );
        Ok(handle)
    }

    fn missing(book: &Book, handle: &WorkerHandle) -> WorkerOutcome {
        let known = handle
            .0
            .strip_prefix(&format!("{}:", book.nonce))
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n > 0 && n <= book.issued);
        outcome(
            if known {
                WorkerOutcomeKind::Closed
            } else {
                WorkerOutcomeKind::Unknown
            },
            None,
        )
    }

    /// Install/execute a bounded plan under the already-known handle.
    /// Identical retries observe the same preparation; conflicting retries fail.
    pub async fn prepare(&self, request: WorkerPrepare) -> WorkerOutcome {
        if let Err(reason) = validate::plan(&request.plan) {
            return outcome(WorkerOutcomeKind::Invalid, Some(reason));
        }
        let mut state = {
            let mut book = self.book.lock().await;
            if book.terminal {
                return outcome(WorkerOutcomeKind::Closed, Some("manager_shutdown"));
            }
            let Some(slot) = book.slots.get_mut(&request.handle) else {
                return Self::missing(&book, &request.handle);
            };
            if slot.state.borrow().closed || *slot.stop.borrow() {
                return outcome(WorkerOutcomeKind::Closed, None);
            }
            if let Some(plan) = &slot.plan {
                if **plan != request.plan {
                    return outcome(WorkerOutcomeKind::Invalid, Some("plan_conflict"));
                }
            } else {
                if slot.reserved_at.elapsed() >= RESERVATION_LIFETIME {
                    return outcome(WorkerOutcomeKind::Closed, Some("reservation_expired"));
                }
                let plan = Arc::new(request.plan);
                let (tx, rx) = mpsc::channel(QUEUE);
                let root = self.root.join(request.handle.0.replace(':', "_"));
                slot.plan = Some(plan.clone());
                slot.commands = Some(tx);
                slot.state.send_replace(State::new(WorkerPhase::Preparing));
                tokio::spawn(actor::run(
                    plan,
                    root,
                    rx,
                    slot.stop.subscribe(),
                    slot.state.clone(),
                ));
            }
            slot.state.subscribe()
        };
        loop {
            let snapshot = state.borrow_and_update().clone();
            if snapshot.closed {
                return outcome(WorkerOutcomeKind::Closed, None);
            }
            if let Some(prepared) = snapshot.prepared {
                return prepared;
            }
            if state.changed().await.is_err() {
                return outcome(WorkerOutcomeKind::Failed, Some("supervisor_closed"));
            }
        }
    }

    async fn submit(
        &self,
        handle: &WorkerHandle,
        command: Command,
        response: oneshot::Receiver<WorkerOutcome>,
    ) -> WorkerOutcome {
        {
            let book = self.book.lock().await;
            if book.terminal {
                return outcome(WorkerOutcomeKind::Closed, Some("manager_shutdown"));
            }
            let Some(slot) = book.slots.get(handle) else {
                return Self::missing(&book, handle);
            };
            if *slot.stop.borrow() || slot.state.borrow().closed {
                return outcome(WorkerOutcomeKind::Closed, None);
            }
            let Some(commands) = &slot.commands else {
                return outcome(WorkerOutcomeKind::Invalid, Some("not_prepared"));
            };
            if commands.try_send(command).is_err() {
                return outcome(WorkerOutcomeKind::Busy, Some("worker_queue"));
            }
        }
        response
            .await
            .unwrap_or_else(|_| outcome(WorkerOutcomeKind::Closed, Some("supervisor_closed")))
    }

    /// Start a prepared worker; the detached supervisor owns pending native startup.
    pub async fn start(&self, handle: &WorkerHandle) -> WorkerOutcome {
        let (reply, response) = oneshot::channel();
        self.submit(handle, Command::Start(reply), response).await
    }

    /// Send a bounded method request. Identical operation retries replay terminal replies.
    pub async fn request(&self, request: WorkerRequest) -> WorkerOutcome {
        if let Err(reason) = validate::request(&request) {
            return outcome(WorkerOutcomeKind::Invalid, Some(reason));
        }
        let handle = request.handle.clone();
        let (reply, response) = oneshot::channel();
        let deadline = {
            let book = self.book.lock().await;
            let Some(slot) = book.slots.get(&handle) else {
                return Self::missing(&book, &handle);
            };
            let Some(plan) = &slot.plan else {
                return outcome(WorkerOutcomeKind::Invalid, Some("not_prepared"));
            };
            Instant::now() + Duration::from_millis(plan.request_timeout_ms)
        };
        tokio::time::timeout_at(
            deadline,
            self.submit(
                &handle,
                Command::Request(request, deadline, reply),
                response,
            ),
        )
        .await
        .unwrap_or_else(|_| outcome(WorkerOutcomeKind::Failed, Some("request_timeout")))
    }

    /// Read phase/status without spawning or exposing native resources.
    ///
    /// # Errors
    /// Returns `Closed` for retired identities or `Unknown` for foreign identities.
    pub async fn status(&self, handle: &WorkerHandle) -> Result<WorkerStatus, WorkerOutcome> {
        let book = self.book.lock().await;
        book.slots
            .get(handle)
            .map(|slot| slot.state.borrow().status.clone())
            .ok_or_else(|| Self::missing(&book, handle))
    }

    /// Signal cancellation out of band and wait for real native cleanup.
    pub async fn stop(&self, handle: &WorkerHandle) -> WorkerOutcome {
        let mut state = {
            let mut book = self.book.lock().await;
            let Some(slot) = book.slots.get_mut(handle) else {
                return Self::missing(&book, handle);
            };
            slot.state.send_modify(|s| s.cleanup_failed = false);
            slot.stop.send_replace(true);
            if slot.commands.is_none() {
                book.slots.remove(handle);
                return outcome(WorkerOutcomeKind::Complete, None);
            }
            slot.state.subscribe()
        };
        if !wait_closed(&mut state).await {
            return outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"));
        }
        self.book.lock().await.slots.remove(handle);
        outcome(WorkerOutcomeKind::Complete, None)
    }

    /// Terminal barrier before ABI unload: reject new work and await every supervisor.
    /// Sign-out should use per-handle stop instead when the module will be reused.
    pub async fn shutdown(&self) -> WorkerOutcome {
        let states = {
            let mut book = self.book.lock().await;
            book.terminal = true;
            book.slots
                .values()
                .filter_map(|slot| {
                    slot.state.send_modify(|s| s.cleanup_failed = false);
                    slot.stop.send_replace(true);
                    slot.commands.as_ref().map(|_| slot.state.subscribe())
                })
                .collect::<Vec<_>>()
        };
        let mut complete = true;
        for mut state in states {
            complete &= wait_closed(&mut state).await;
        }
        if complete {
            self.book.lock().await.slots.clear();
            outcome(WorkerOutcomeKind::Complete, None)
        } else {
            outcome(WorkerOutcomeKind::Failed, Some("cleanup_failed"))
        }
    }
}

async fn wait_closed(state: &mut watch::Receiver<State>) -> bool {
    loop {
        let snapshot = state.borrow_and_update().clone();
        if snapshot.closed {
            return true;
        }
        if snapshot.cleanup_failed {
            return false;
        }
        if state.changed().await.is_err() {
            return false;
        }
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
