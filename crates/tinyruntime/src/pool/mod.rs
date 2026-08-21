//! Warm interpreter processes, and the registry that keeps one set per language.
//!
//! ## Why warm workers at all
//!
//! A short-lived interpreter child costs tens of megabytes resident before it
//! runs a line of anyone's code. A host running many agents concurrently pays
//! that per execution, and it is the largest single cost in the whole path. A
//! small bounded set of warm workers turns *K concurrent jobs into K
//! interpreters* into *K concurrent jobs into a handful*, trading a little
//! queueing latency for a flat memory floor.
//!
//! ## The shape
//!
//! - [`protocol`] — the newline-delimited framing, on its own socket so a job's
//!   own output can never be mistaken for a protocol frame.
//! - [`worker`] — one warm child and the rules for trusting it.
//! - [`env`](mod@env) — the allow-listed environment and the harness on disk.
//! - `lang_pool` — the bounded pool: permits, backpressure, recycling, reaping.
//!
//! The registry below keys a pool by its launch fingerprint, so retuning the
//! pool or resolving a different toolchain transparently rebuilds it rather than
//! quietly serving from the old one.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use tinyruntime_bus::{Language, PoolSettings, PoolStats};

pub mod env;
mod lang_pool;
pub mod protocol;
pub mod worker;

pub use lang_pool::LangPool;
pub use worker::{Launch, Worker};

/// The live pools, one per language.
#[derive(Debug, Default)]
pub struct Pools {
    live: Mutex<HashMap<Language, Entry>>,
}

/// One live pool and the fingerprint it was built for.
#[derive(Debug)]
struct Entry {
    fingerprint: String,
    pool: Arc<LangPool>,
}

impl Pools {
    /// An empty set of pools.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The pool for this launch, building it if the current one does not match.
    ///
    /// A mismatch means the interpreter, its flags, its environment, or the pool
    /// tuning changed. Reusing warm workers across that would answer from the
    /// wrong toolchain, so the old pool is dropped — its workers die with it —
    /// and a new one takes over.
    pub async fn ensure(
        &self,
        launch: Launch,
        settings: PoolSettings,
        runtime_version: String,
    ) -> Arc<LangPool> {
        let fingerprint = format!(
            "{}|workers={}|ttl={}|recycle={}|queue={}",
            launch.fingerprint(),
            settings.effective_max_workers(),
            settings.idle_ttl_secs,
            settings.recycle_after_jobs,
            settings.max_queue_depth,
        );
        let language = launch.language.clone();

        let mut live = self.live.lock().await;
        if let Some(entry) = live.get(&language) {
            if entry.fingerprint == fingerprint {
                return Arc::clone(&entry.pool);
            }
            tracing::info!(
                language = language.as_str(),
                "[tinyruntime::pool] the launch changed; rebuilding the pool"
            );
        }

        let pool = LangPool::start(launch, settings, runtime_version);
        live.insert(
            language,
            Entry {
                fingerprint,
                pool: Arc::clone(&pool),
            },
        );
        pool
    }

    /// Every live pool's counters, in language order.
    pub async fn stats(&self) -> Vec<PoolStats> {
        let live = self.live.lock().await;
        let mut entries: Vec<&Entry> = live.values().collect();
        entries.sort_by(|left, right| {
            left.pool
                .launch()
                .language
                .cmp(&right.pool.launch().language)
        });

        let mut stats = Vec::with_capacity(entries.len());
        for entry in entries {
            stats.push(entry.pool.stats().await);
        }
        stats
    }
}

#[cfg(test)]
#[path = "pool_test.rs"]
mod test;
