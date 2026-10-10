//! Legacy reexports of the router-owned JSONL vocabulary.
pub use tinyruntime_bus::worker::{
    PROTOCOL_VERSION, ReadyLine, ServerError, ServerRequest, ServerResponse,
};

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
