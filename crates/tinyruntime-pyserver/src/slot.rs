//! A process-wide cache of one worker, with startup back-off.
//!
//! A host wants exactly one warm worker per process, shared by every caller,
//! rebuilt when the backends it must serve change, and *not* re-launched in a
//! tight loop when its virtual environment is broken. [`ServerSlot`] is that
//! policy, with the launch itself left to the host: the slot is handed a
//! future that prepares a [`ServerLaunch`] and only runs it when a worker has to
//! be built.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::launch::ServerLaunch;
use crate::server::PythonServer;
use crate::status::ServerStatus;

/// How long a failed startup is remembered before it is retried.
pub const START_FAILURE_BACKOFF: Duration = Duration::from_secs(300);

/// When a running worker should be torn down for being idle.
///
/// Only the named backend's idleness counts: a heavyweight backend (a torch
/// model) is worth freeing after a quiet spell, while a light one is not worth
/// a restart.
#[derive(Debug, Clone, Copy)]
pub struct IdleRule<'a> {
    /// The backend id the rule applies to.
    pub backend: &'a str,
    /// How long without a successful request counts as idle.
    pub timeout: Duration,
}

#[derive(Debug, Clone)]
enum Cache {
    Empty,
    Ready(Arc<PythonServer>),
    Failed {
        message: String,
        retry_after: Instant,
    },
}

/// The host's launch preparation, boxed so the policy below is compiled once
/// rather than once per caller.
type Launching<'a> = Pin<Box<dyn Future<Output = Result<ServerLaunch>> + Send + 'a>>;

/// One shared worker and the policy for keeping it.
#[derive(Debug)]
pub struct ServerSlot {
    cache: Mutex<Cache>,
}

impl Default for ServerSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerSlot {
    /// An empty slot. `const`, so a host can hold it in a `static`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            cache: Mutex::const_new(Cache::Empty),
        }
    }

    /// The running worker, building it first when there is none.
    ///
    /// * A cached worker launched for a different `requested` backend set is
    ///   discarded and rebuilt: it was never prepared for the new backend.
    /// * A cached worker idle past `idle` is discarded and rebuilt.
    /// * A failed startup is remembered for [`START_FAILURE_BACKOFF`], during
    ///   which callers get the previous failure back instead of a new attempt.
    ///
    /// `prepare` is awaited only when a new worker must be built; it is a lazy
    /// future, so anything it would do (provisioning a virtual environment) is
    /// skipped when a worker is already running.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unavailable`] when a worker could not be started,
    /// [`Error::BackingOff`] while a previous failure is being honoured.
    pub async fn ensure<F>(
        &self,
        requested: &[String],
        idle: Option<IdleRule<'_>>,
        prepare: F,
    ) -> Result<Arc<PythonServer>>
    where
        F: Future<Output = Result<ServerLaunch>> + Send,
    {
        self.ensure_boxed(requested, idle, Box::pin(prepare)).await
    }

    async fn ensure_boxed(
        &self,
        requested: &[String],
        idle: Option<IdleRule<'_>>,
        prepare: Launching<'_>,
    ) -> Result<Arc<PythonServer>> {
        let mut guard = self.cache.lock().await;
        let cached = match &*guard {
            Cache::Ready(existing) => Some(existing.clone()),
            Cache::Empty | Cache::Failed { .. } => None,
        };
        if let Some(existing) = cached {
            if existing.backends() != requested {
                tracing::info!(
                    "[runtime_python_server] backend set changed ({:?} -> {:?}); rebuilding server",
                    existing.backends(),
                    requested
                );
                existing.reset().await;
                *guard = Cache::Empty;
            } else if let Some(rule) = idle
                && existing.backends().iter().any(|id| id == rule.backend)
                && existing.idle_expired(rule.timeout).await
            {
                tracing::info!(
                    "[runtime_python_server] {} backend idle for >= {:?}; rebuilding server",
                    rule.backend,
                    rule.timeout
                );
                existing.reset().await;
                *guard = Cache::Empty;
            }
        }
        match &*guard {
            Cache::Ready(existing) => {
                let existing = existing.clone();
                if let Err(error) = existing.start().await {
                    let message = error.to_string();
                    tracing::warn!(
                        "[runtime_python_server] cached server failed to start; backing off: {message}"
                    );
                    *guard = failed(message.clone());
                    return Err(Error::Unavailable(message));
                }
                return Ok(existing);
            }
            Cache::Failed {
                message,
                retry_after,
            } if Instant::now() < *retry_after => {
                return Err(Error::BackingOff(message.clone()));
            }
            Cache::Failed { .. } | Cache::Empty => {}
        }

        match build(prepare).await {
            Ok(server) => {
                *guard = Cache::Ready(server.clone());
                Ok(server)
            }
            Err(error) => {
                let message = error.to_string();
                tracing::warn!(
                    "[runtime_python_server] startup failed; caching fallback state for {START_FAILURE_BACKOFF:?}: {message}"
                );
                *guard = failed(message.clone());
                Err(Error::Unavailable(message))
            }
        }
    }

    /// The cached worker's state, without starting anything.
    pub async fn status(&self) -> ServerStatus {
        let cached = self.cache.lock().await.clone();
        match cached {
            Cache::Ready(server) => server.status().await,
            Cache::Failed { message, .. } => ServerStatus {
                enabled: true,
                running: false,
                backends: Vec::new(),
                message: Some(format!("runtime python server unavailable: {message}")),
            },
            Cache::Empty => ServerStatus::disabled("runtime python server has not started"),
        }
    }
}

fn failed(message: String) -> Cache {
    Cache::Failed {
        message,
        retry_after: Instant::now() + START_FAILURE_BACKOFF,
    }
}

async fn build(prepare: Launching<'_>) -> Result<Arc<PythonServer>> {
    let server = Arc::new(PythonServer::new(prepare.await?));
    server.start().await?;
    Ok(server)
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
