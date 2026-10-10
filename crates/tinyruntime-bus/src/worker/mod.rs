//! Generic persistent process plans and legacy JSONL/status vocabulary.
mod cache;
pub use cache::{
    CacheAdoptionPolicy, CacheArtifact, CacheEntry, CacheEntryKind, CacheRecipe, WorkerCacheScope,
    WorkerPrepareCached,
};
mod protocol;
mod status;
mod types;
pub use protocol::{PROTOCOL_VERSION, ReadyLine, ServerError, ServerRequest, ServerResponse};
pub use status::{BackendStatus, ServerStatus};
pub use types::*;
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
