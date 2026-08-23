//! The framing between the router and a warm worker.
//!
//! Newline-delimited JSON over a duplex stream: one handshake line from the
//! worker, then one request line and one response line per job, correlated by
//! id.
//!
//! The stream is deliberately *not* the child's standard input and output. A job
//! writes to those, and a job that prints a line shaped like a response frame
//! would otherwise be able to answer its own request — or desynchronise the next
//! one. The worker therefore connects back over an authenticated loopback socket
//! and its own stdout is drained and logged, never parsed.

use serde::{Deserialize, Serialize};

/// The handshake a worker prints exactly once on startup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Handshake {
    /// Whether the worker came up.
    #[serde(default)]
    pub ready: bool,
    /// The protocol version the worker implements.
    #[serde(default)]
    pub protocol: Option<u32>,
    /// The language the worker believes it is serving, as a sanity check that
    /// the right harness was launched under the right interpreter.
    #[serde(default)]
    pub language: Option<String>,
    /// Why the worker did not come up, when it did not.
    #[serde(default)]
    pub error: Option<String>,
    /// The per-launch secret, echoed back to prove this connection came from the
    /// process the router just spawned rather than from anything else that
    /// happened to reach the loopback port.
    #[serde(default)]
    pub token: Option<String>,
}

/// One unit of work sent to a worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRequest {
    /// Correlation id the worker echoes back.
    pub id: String,
    /// The source to evaluate.
    pub code: String,
    /// Working directory the worker changes to before running the job.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Soft deadline in milliseconds; absent means run to completion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

/// A worker's reply to one [`JobRequest`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobResponse {
    /// The id of the request this answers.
    pub id: Option<String>,
    /// Whether the harness ran the job to a conclusion.
    ///
    /// The job's own code may still have thrown — that shows up in `exit_code`
    /// and `stderr`. This is `false` only when the harness itself could not run
    /// the job at all, and then `error` says why.
    #[serde(default)]
    pub ok: bool,
    /// What the job wrote to standard output.
    #[serde(default)]
    pub stdout: String,
    /// What the job wrote to standard error.
    #[serde(default)]
    pub stderr: String,
    /// `0` on a clean run, non-zero when the job threw or exited non-zero.
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// Whether the worker aborted the job at its soft deadline.
    #[serde(default)]
    pub timed_out: bool,
    /// How long the job took inside the worker.
    #[serde(default)]
    pub elapsed_ms: u64,
    /// A harness-level failure, when the worker could not run the job.
    #[serde(default)]
    pub error: Option<String>,
}

#[cfg(test)]
#[path = "protocol_test.rs"]
mod test;
