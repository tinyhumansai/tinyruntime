//! The warm-worker pool's tuning knobs and its counters.

use serde::{Deserialize, Serialize};

use crate::Language;

/// How a host wants one language's worker pool sized and recycled.
///
/// The pool exists because a per-execution interpreter child costs tens of
/// megabytes resident, and a host running many concurrent jobs pays that per
/// job. A small bounded set of warm workers turns *K concurrent jobs into K
/// interpreters* into *K concurrent jobs into a handful*, trading queueing
/// latency for a flat memory floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PoolSettings {
    /// Whether execution may use warm workers at all. With the pool off, every
    /// job gets its own short-lived interpreter child.
    pub enabled: bool,
    /// Concurrent workers. Jobs beyond this queue rather than spawning.
    pub max_workers: usize,
    /// Retire a worker after this long idle. `0` keeps warm workers forever.
    pub idle_ttl_secs: u64,
    /// Retire a worker after this many jobs, bounding cross-job state leakage.
    /// `0` disables recycling.
    pub recycle_after_jobs: u64,
    /// Queued jobs allowed beyond the worker slots before the pool sheds load.
    pub max_queue_depth: usize,
}

impl PoolSettings {
    /// The clamped worker count. Never zero: a pool that can hold no workers
    /// would deadlock every submission rather than merely disabling itself.
    #[must_use]
    pub fn effective_max_workers(&self) -> usize {
        self.max_workers.max(1)
    }

    /// The clamped queue depth, allowing a queue of zero (submissions beyond the
    /// worker slots are shed immediately).
    #[must_use]
    pub fn effective_max_queue_depth(&self) -> usize {
        self.max_queue_depth
    }

    /// The idle time-to-live, or `None` when warm workers are never reaped.
    #[must_use]
    pub fn idle_ttl_secs(&self) -> Option<u64> {
        if self.idle_ttl_secs == 0 {
            None
        } else {
            Some(self.idle_ttl_secs)
        }
    }
}

impl Default for PoolSettings {
    /// A small pool sized for a host running many agents rather than one big job.
    fn default() -> Self {
        Self {
            enabled: true,
            max_workers: 2,
            idle_ttl_secs: 300,
            recycle_after_jobs: 100,
            max_queue_depth: 256,
        }
    }
}

/// One live pool's counters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PoolStats {
    /// The language this pool serves.
    pub language: Language,
    /// Jobs completed since the pool was built.
    pub jobs_total: u64,
    /// Interpreter children spawned. Far below `jobs_total` is the pool working.
    pub worker_spawns: u64,
    /// Submissions refused because the pool was at capacity.
    pub rejected_saturated: u64,
    /// Warm workers currently parked and reusable.
    pub idle_workers: usize,
    /// The pool's configured concurrency.
    pub max_workers: usize,
}

/// The reply listing every live pool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PoolStatsResponse {
    /// One entry per language with a live pool.
    pub pools: Vec<PoolStats>,
}
