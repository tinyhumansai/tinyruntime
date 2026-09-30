//! A persistent Python worker host.
//!
//! # What this crate is for
//!
//! Some Python-backed capabilities (a spaCy pipeline, a torch model) are too
//! slow to start to pay for on every call. This crate keeps one interpreter
//! process warm and talks to it over a private JSON-lines protocol on its
//! stdin and stdout: a handshake line, then one response per request.
//!
//! It is the in-process counterpart of the `tinyruntime` module's job pool: no
//! bus, no loopback socket, no per-job harness. A host that wants sandboxed
//! one-shot execution uses the module; a host that wants a resident model links
//! this.
//!
//! # The parts
//!
//! * [`PythonServer`] is one worker: spawn, handshake, request with one
//!   automatic restart-and-retry, status, idle check.
//! * [`ServerSlot`] is the process-wide cache around it: one shared worker,
//!   rebuilt when the backend set changes or a heavyweight backend goes idle,
//!   with a five-minute back-off after a failed start.
//! * [`ServerLaunch`] is everything the host decides: interpreter, script,
//!   backends, environment, timeouts.
//! * [`SERVER_SCRIPT`] and [`write_script`] are the worker itself, embedded so
//!   its protocol revision always matches this crate's.
//! * [`protocol`] and [`ServerStatus`] are the wire and status types.
//!
//! # What it does not hold
//!
//! Choosing or installing the interpreter, building a virtual environment and
//! installing packages into it, and mapping a host's configuration onto
//! backends and environment variables all stay with the host. This crate is
//! handed a finished [`ServerLaunch`] and never reads a host setting.

mod error;
mod launch;
pub mod protocol;
mod script;
mod server;
mod slot;
mod status;
#[cfg(test)]
mod testing;

pub use error::{Error, Result};
pub use launch::{DEFAULT_HANDSHAKE_TIMEOUT, DEFAULT_REQUEST_TIMEOUT, ServerLaunch};
pub use protocol::PROTOCOL_VERSION;
pub use script::{SCRIPT_FILE_NAME, SERVER_SCRIPT, write_script};
pub use server::PythonServer;
pub use slot::{IdleRule, START_FAILURE_BACKOFF, ServerSlot};
pub use status::{BackendStatus, ServerStatus};
