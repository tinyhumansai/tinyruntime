//! Requests and replies for running code on a resolved runtime.

use serde::{Deserialize, Serialize};

use crate::{Language, PoolSettings, RuntimeSettings};

/// Run inline source on a language runtime.
///
/// The request carries everything the router needs to resolve, provision, and
/// run in one call. A host that wants the resolution without the execution asks
/// for it separately; a host that just wants the answer should not have to make
/// two round trips and hold the toolchain state itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecRequest {
    /// Which language the source is written in.
    pub language: Language,
    /// How to resolve the toolchain that runs it.
    pub settings: RuntimeSettings,
    /// How to size the warm-worker pool that hosts the job.
    pub pool: PoolSettings,
    /// The source to evaluate.
    pub code: String,
    /// Working directory for the job.
    ///
    /// A job with no working directory runs wherever the worker happens to be,
    /// which for a shared warm worker is not a place a caller can reason about.
    /// Callers that care about relative paths must set it.
    pub cwd: Option<String>,
    /// Soft deadline in milliseconds. The worker aborts the job when it elapses
    /// and still replies; absent means run to completion.
    pub timeout_ms: Option<u64>,
}

impl ExecRequest {
    /// Builds a request to run `code`, with default pool tuning.
    #[must_use]
    pub fn new(language: Language, settings: RuntimeSettings, code: impl Into<String>) -> Self {
        Self {
            language,
            settings,
            pool: PoolSettings::default(),
            code: code.into(),
            cwd: None,
            timeout_ms: None,
        }
    }

    /// Sets the job's working directory.
    #[must_use]
    pub fn with_cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Sets the job's soft deadline.
    #[must_use]
    pub fn with_timeout_ms(mut self, timeout_ms: u64) -> Self {
        self.timeout_ms = Some(timeout_ms);
        self
    }
}

/// What running the job produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExecResponse {
    /// Everything the job wrote to standard output.
    pub stdout: String,
    /// Everything the job wrote to standard error.
    pub stderr: String,
    /// `0` when the job completed cleanly, non-zero when it threw or exited
    /// non-zero, `None` when the concept does not apply.
    pub exit_code: Option<i32>,
    /// Whether the job was aborted at its soft deadline.
    pub timed_out: bool,
    /// Wall-clock the job itself took.
    pub elapsed_ms: u64,
    /// How long the submission waited for a free worker.
    ///
    /// Reported separately from `elapsed_ms` so a host can tell a slow job from a
    /// busy pool — they call for opposite responses.
    pub queue_wait_ms: u64,
    /// The version of the toolchain that ran the job.
    pub runtime_version: String,
}

impl ExecResponse {
    /// Whether the job ran to completion without throwing or timing out.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinyruntime_bus::ExecResponse;
    /// let mut response = ExecResponse::new("42\n", "", Some(0), "22.11.0");
    /// assert!(response.success());
    /// response.timed_out = true;
    /// assert!(!response.success());
    /// ```
    #[must_use]
    pub fn success(&self) -> bool {
        !self.timed_out && matches!(self.exit_code, None | Some(0))
    }

    /// Records how long the job ran and how long it waited for a worker.
    #[must_use]
    pub fn with_timings(mut self, elapsed_ms: u64, queue_wait_ms: u64) -> Self {
        self.elapsed_ms = elapsed_ms;
        self.queue_wait_ms = queue_wait_ms;
        self
    }

    /// Marks the job as aborted at its soft deadline.
    #[must_use]
    pub fn with_timed_out(mut self, timed_out: bool) -> Self {
        self.timed_out = timed_out;
        self
    }

    /// Builds a response with zeroed timings.
    #[must_use]
    pub fn new(
        stdout: impl Into<String>,
        stderr: impl Into<String>,
        exit_code: Option<i32>,
        runtime_version: impl Into<String>,
    ) -> Self {
        Self {
            stdout: stdout.into(),
            stderr: stderr.into(),
            exit_code,
            timed_out: false,
            elapsed_ms: 0,
            queue_wait_ms: 0,
            runtime_version: runtime_version.into(),
        }
    }
}
