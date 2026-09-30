//! The crate-wide error type.
//!
//! Every message is complete on its own: the worker is a black box to its
//! caller, so an error has to say which step of talking to it failed and carry
//! the underlying cause in its text rather than as a chain to walk.

use std::io;

/// A convenience alias for this crate's fallible operations.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Why starting or talking to the worker failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The interpreter could not be spawned.
    #[error(
        "spawning runtime python server: failed to spawn python process `{bin}` for script {script}: {source}"
    )]
    Spawn {
        /// The interpreter that was launched.
        bin: String,
        /// The worker script it was given.
        script: String,
        /// The operating system's reason.
        source: io::Error,
    },
    /// The child had no stdin pipe.
    #[error("runtime python server stdin missing")]
    StdinMissing,
    /// The child had no stdout pipe.
    #[error("runtime python server stdout missing")]
    StdoutMissing,
    /// The child exited before printing its ready line.
    #[error("runtime python server exited before readiness handshake")]
    ExitedBeforeReady,
    /// Reading the ready line failed.
    #[error("reading runtime python server handshake: {0}")]
    HandshakeRead(io::Error),
    /// The ready line did not arrive in time.
    #[error("runtime python server readiness handshake timed out")]
    HandshakeTimeout,
    /// The first line was not a ready line.
    #[error("parsing runtime python server ready line: {line}: {source}")]
    ReadyParse {
        /// The line the worker printed.
        line: String,
        /// Why it did not parse.
        source: serde_json::Error,
    },
    /// The worker reported that it could not start.
    #[error("runtime python server failed to start: {0}")]
    StartFailed(String),
    /// The worker speaks a different protocol revision.
    #[error("runtime python server protocol mismatch: expected {expected}, got {got:?}")]
    ProtocolMismatch {
        /// The revision this crate speaks.
        expected: u32,
        /// The revision the worker reported.
        got: Option<u32>,
    },
    /// A request could not be encoded.
    #[error("encoding runtime python server request: {0}")]
    Encode(serde_json::Error),
    /// A request could not be written.
    #[error("writing runtime python server request: {0}")]
    Write(io::Error),
    /// A request could not be flushed.
    #[error("flushing runtime python server request: {0}")]
    Flush(io::Error),
    /// The worker closed its stdout while a request was outstanding.
    #[error("runtime python server closed stdout")]
    Closed,
    /// Reading a response failed.
    #[error("reading runtime python server response: {0}")]
    Read(io::Error),
    /// No response arrived within the request timeout.
    #[error("runtime python server request timed out")]
    RequestTimeout,
    /// The worker answered with a failure.
    #[error("runtime python server `{method}` failed: {message}")]
    Remote {
        /// The method that failed.
        method: String,
        /// `code: message` as the worker reported it.
        message: String,
    },
    /// The worker's result did not match the type the caller expected.
    #[error("decoding runtime python server `{method}` result: {source}")]
    Decode {
        /// The method whose result was rejected.
        method: String,
        /// Why it did not decode.
        source: serde_json::Error,
    },
    /// No worker is available; startup failed just now.
    #[error("runtime python server unavailable: {0}")]
    Unavailable(String),
    /// No worker is available; startup failed recently and is not retried yet.
    #[error("runtime python server unavailable after previous startup failure: {0}")]
    BackingOff(String),
    /// The host could not prepare a launch; carries the host's own message.
    #[error("{0}")]
    Prepare(String),
    /// The script directory could not be created.
    #[error("creating runtime python server cache {path}: {source}")]
    CreateDir {
        /// The directory.
        path: String,
        /// The operating system's reason.
        source: io::Error,
    },
    /// The script could not be written.
    #[error("writing runtime python server script {path}: {source}")]
    WriteScript {
        /// The file.
        path: String,
        /// The operating system's reason.
        source: io::Error,
    },
}
