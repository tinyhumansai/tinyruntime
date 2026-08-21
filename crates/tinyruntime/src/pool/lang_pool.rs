//! The bounded pool of warm workers for one language.
//!
//! Concurrency is a semaphore with one permit per worker slot. A submission
//! beyond the slots waits on it — that queueing *is* the backpressure — and a
//! submission beyond the slots plus the allowed queue depth is refused outright.
//!
//! Refusing rather than queueing without limit is the point. A host under a
//! stampede that buffered every job would trade the memory the pool saves for
//! memory spent on the queue, and would do it invisibly. A refusal is a signal
//! the host can act on, which is why [`crate::error::Error::PoolSaturated`] tells
//! callers not to fall back to spawning their own interpreter.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Semaphore};

use tinyruntime_bus::{ExecResponse, PoolSettings, PoolStats};

use super::protocol::{JobRequest, JobResponse};
use super::worker::{Launch, Worker};
use crate::error::{Error, Result};

/// Grace added to a job's soft deadline before the worker is treated as wedged.
///
/// The worker aborts at the soft deadline and still replies, so this only fires
/// when the worker itself has stopped answering — at which point it is killed
/// and replaced rather than waited on.
const WEDGED_GRACE: Duration = Duration::from_secs(10);

/// Floor on how often the idle reaper wakes, so a small time-to-live does not
/// turn into a busy loop.
const MIN_REAP_INTERVAL: Duration = Duration::from_secs(5);

/// A bounded set of warm workers for one language.
#[derive(Debug)]
pub struct LangPool {
    launch: Launch,
    settings: PoolSettings,
    runtime_version: String,
    permits: Arc<Semaphore>,
    idle: Mutex<Vec<Worker>>,
    inflight: AtomicUsize,
    next_job: AtomicU64,
    jobs_total: AtomicU64,
    worker_spawns: AtomicU64,
    rejected: AtomicU64,
}

impl LangPool {
    /// Build a pool and start its idle reaper.
    #[must_use]
    pub fn start(launch: Launch, settings: PoolSettings, runtime_version: String) -> Arc<Self> {
        let pool = Arc::new(Self {
            permits: Arc::new(Semaphore::new(settings.effective_max_workers())),
            launch,
            settings,
            runtime_version,
            idle: Mutex::new(Vec::new()),
            inflight: AtomicUsize::new(0),
            next_job: AtomicU64::new(0),
            jobs_total: AtomicU64::new(0),
            worker_spawns: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
        });
        if let Some(ttl) = pool.settings.idle_ttl_secs() {
            // The reaper holds a weak reference so a pool that has been replaced
            // is dropped normally instead of being kept alive by its own timer.
            spawn_reaper(
                Arc::downgrade(&pool),
                Duration::from_secs(ttl).max(MIN_REAP_INTERVAL),
            );
        }
        pool
    }

    /// The launch this pool was built for.
    #[must_use]
    pub fn launch(&self) -> &Launch {
        &self.launch
    }

    /// This pool's counters.
    pub async fn stats(&self) -> PoolStats {
        PoolStats::new(
            self.launch.language.clone(),
            self.settings.effective_max_workers(),
        )
        .with_counts(
            self.jobs_total.load(Ordering::Relaxed),
            self.worker_spawns.load(Ordering::Relaxed),
            self.rejected.load(Ordering::Relaxed),
        )
        .with_idle_workers(self.idle.lock().await.len())
    }

    /// Run one job, waiting for a free worker.
    ///
    /// # Errors
    ///
    /// Returns [`Error::PoolSaturated`] when the pool is at capacity, and the
    /// dispatch variants when the job could not be run.
    pub async fn run(
        &self,
        code: String,
        cwd: Option<String>,
        timeout: Option<Duration>,
    ) -> Result<ExecResponse> {
        let capacity =
            self.settings.effective_max_workers() + self.settings.effective_max_queue_depth();
        if self.inflight.fetch_add(1, Ordering::AcqRel) + 1 > capacity {
            self.inflight.fetch_sub(1, Ordering::AcqRel);
            self.rejected.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                language = self.launch.language.as_str(),
                capacity,
                "[tinyruntime::pool] at capacity; shedding load"
            );
            return Err(Error::PoolSaturated(self.launch.language.clone()));
        }
        let _inflight = InflightGuard(&self.inflight);

        let queued_at = Instant::now();
        let Ok(_permit) = self.permits.acquire().await else {
            // The semaphore is never closed, so this is unreachable in practice;
            // reporting it beats a panic in a module inside someone's process.
            return Err(Error::PreDispatch {
                language: self.launch.language.clone(),
                reason: "the pool was shut down".to_string(),
            });
        };
        let queue_wait = queued_at.elapsed();

        let request = JobRequest {
            id: self.next_job.fetch_add(1, Ordering::Relaxed).to_string(),
            code,
            cwd,
            timeout_ms: timeout.map(|budget| u64::try_from(budget.as_millis()).unwrap_or(u64::MAX)),
        };
        let hard_deadline = timeout.map(|budget| budget + WEDGED_GRACE);

        let started = Instant::now();
        let (response, worker) = self.dispatch(&request, hard_deadline).await?;
        let elapsed = started.elapsed();
        self.jobs_total.fetch_add(1, Ordering::Relaxed);

        self.retire_or_park(worker).await;

        if let Some(error) = response.error {
            // The harness reported a failure of its own. The job reached the
            // worker, so this is terminal rather than retryable.
            return Err(Error::PostDispatch {
                language: self.launch.language.clone(),
                reason: error,
            });
        }

        Ok(self.outcome(response, elapsed, queue_wait))
    }

    /// Assemble the reply from a worker's response and the router's timings.
    fn outcome(
        &self,
        response: JobResponse,
        elapsed: Duration,
        queue_wait: Duration,
    ) -> ExecResponse {
        ExecResponse::new(
            response.stdout,
            response.stderr,
            response.exit_code,
            self.runtime_version.clone(),
        )
        .with_timed_out(response.timed_out)
        .with_timings(
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            u64::try_from(queue_wait.as_millis()).unwrap_or(u64::MAX),
        )
    }

    /// Submit on a warm or fresh worker, respawning once if the job provably
    /// never left.
    ///
    /// The retry is allowed exactly once and exactly for a pre-dispatch failure.
    /// A parked worker can die between jobs, and refusing to retry that would
    /// turn an idle timeout on the far side into a user-visible failure — but a
    /// failure after the request went out may mean the job already ran, and
    /// running it again could duplicate whatever it did.
    async fn dispatch(
        &self,
        request: &JobRequest,
        hard_deadline: Option<Duration>,
    ) -> Result<(JobResponse, Worker)> {
        let mut worker = self.take_or_spawn().await?;
        match worker.submit(request, hard_deadline).await {
            Ok(response) => Ok((response, worker)),
            Err(failure) if !failure.dispatched => {
                tracing::warn!(
                    language = self.launch.language.as_str(),
                    "[tinyruntime::pool] the job never left ({}); respawning once",
                    failure.reason
                );
                worker.shutdown();
                let mut fresh = self.spawn().await?;
                match fresh.submit(request, hard_deadline).await {
                    Ok(response) => Ok((response, fresh)),
                    Err(second) => {
                        fresh.shutdown();
                        Err(self.dispatch_error(&second))
                    }
                }
            }
            Err(failure) => {
                worker.shutdown();
                Err(self.dispatch_error(&failure))
            }
        }
    }

    /// Classify a submit failure by whether the job may have run.
    fn dispatch_error(&self, failure: &super::worker::SubmitFailure) -> Error {
        let language = self.launch.language.clone();
        let reason = failure.reason.clone();
        if failure.dispatched {
            Error::PostDispatch { language, reason }
        } else {
            Error::PreDispatch { language, reason }
        }
    }

    /// Recycle a worker that has served its budget, or park it for reuse.
    async fn retire_or_park(&self, worker: Worker) {
        if worker.should_recycle(self.settings.recycle_after_jobs) {
            tracing::debug!(
                language = self.launch.language.as_str(),
                jobs = worker.jobs_done(),
                "[tinyruntime::pool] recycling a worker that served its budget"
            );
            worker.shutdown();
        } else {
            self.idle.lock().await.push(worker);
        }
    }

    /// Take a still-usable parked worker, or spawn one.
    ///
    /// A parked worker is discarded when it is past its time-to-live, and also
    /// when its child has already exited — see [`Worker::has_exited`] for why
    /// that second check is what keeps a job that never ran from being failed
    /// as though it might have.
    async fn take_or_spawn(&self) -> Result<Worker> {
        {
            let mut idle = self.idle.lock().await;
            while let Some(mut worker) = idle.pop() {
                if let Some(ttl) = self.settings.idle_ttl_secs()
                    && worker.idle_expired(Duration::from_secs(ttl))
                {
                    worker.shutdown();
                    continue;
                }
                if worker.has_exited() {
                    tracing::debug!(
                        language = self.launch.language.as_str(),
                        "[tinyruntime::pool] a parked worker had died; replacing it"
                    );
                    worker.shutdown();
                    continue;
                }
                return Ok(worker);
            }
        }
        self.spawn().await
    }

    /// Start a new worker.
    async fn spawn(&self) -> Result<Worker> {
        self.worker_spawns.fetch_add(1, Ordering::Relaxed);
        Worker::spawn(&self.launch)
            .await
            .map_err(|reason| Error::PreDispatch {
                language: self.launch.language.clone(),
                reason,
            })
    }

    /// Retire parked workers that have been idle beyond the time-to-live.
    ///
    /// Visible to the module's tests so the reaper can be driven directly. The
    /// alternative is a test that sleeps past [`MIN_REAP_INTERVAL`], which would
    /// add five seconds to the suite to observe something this call decides.
    pub(super) async fn reap(&self) {
        let Some(ttl) = self.settings.idle_ttl_secs().map(Duration::from_secs) else {
            return;
        };
        let mut idle = self.idle.lock().await;
        let before = idle.len();
        let mut kept = Vec::with_capacity(before);
        for worker in idle.drain(..) {
            if worker.idle_expired(ttl) {
                worker.shutdown();
            } else {
                kept.push(worker);
            }
        }
        let reaped = before - kept.len();
        *idle = kept;
        if reaped > 0 {
            tracing::debug!(
                language = self.launch.language.as_str(),
                reaped,
                "[tinyruntime::pool] retired idle workers"
            );
        }
    }
}

/// Releases the in-flight count on every exit path, including the early returns.
struct InflightGuard<'a>(&'a AtomicUsize);

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Run the idle reaper until the pool it watches is dropped.
///
/// Takes the interval rather than deriving it, so a test can run the loop at a
/// pace that does not add [`MIN_REAP_INTERVAL`] to the suite.
pub(super) fn spawn_reaper(pool: Weak<LangPool>, interval: Duration) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(interval).await;
            match pool.upgrade() {
                Some(pool) => pool.reap().await,
                None => break,
            }
        }
    });
}
