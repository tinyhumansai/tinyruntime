//! The worker script a provider ships and the router runs.
//!
//! Shipping the harness through the contract, rather than compiling it into the
//! router, is what keeps the router language-agnostic: it writes bytes to a
//! file, launches an interpreter, and speaks one framing. What those bytes do —
//! isolate a job, capture its output, abort it at a deadline — is the provider's
//! problem, in the provider's language.

mod types;

pub use types::{WORKER_PROTOCOL_VERSION, WorkerHarness};

#[cfg(test)]
mod test;
