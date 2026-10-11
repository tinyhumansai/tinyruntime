//! Legacy reexports of router-owned persistent worker status.
pub use tinyruntime_bus::worker::{BackendStatus, ServerStatus};

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
