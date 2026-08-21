//! Tuning and observing the warm-worker pool.
//!
//! Pool settings travel with an execution request for the same reason runtime
//! settings do: the module holds no configuration of its own, and a host that
//! retunes its pool sees the change on the next call. A settings change the pool
//! cannot absorb in place rebuilds it transparently.

mod types;

pub use types::{PoolSettings, PoolStats, PoolStatsResponse};

#[cfg(test)]
mod test;
