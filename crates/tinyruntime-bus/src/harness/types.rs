//! The worker harness a provider ships for the router to launch.

use serde::{Deserialize, Serialize};

/// The newline-delimited JSON protocol version the router speaks to workers.
///
/// A worker announces the version it implements in its handshake; a mismatch
/// fails the launch rather than letting two incompatible framings talk past each
/// other one job at a time.
pub const WORKER_PROTOCOL_VERSION: u32 = 1;

/// The script that turns an interpreter into a warm worker, and how to launch it.
///
/// The provider owns this because only it knows how its language isolates a job,
/// captures output, and honours a deadline. The router owns everything around
/// it: writing the script out, building the command, completing the handshake,
/// and keeping the worker warm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WorkerHarness {
    /// Filename to materialise the script under, e.g. `pool_worker.js`.
    pub filename: String,
    /// The script source.
    pub source: String,
    /// Interpreter flags placed before the script path.
    pub args_before_script: Vec<String>,
    /// Arguments placed after the script path.
    pub args_after_script: Vec<String>,
    /// Extra environment for the worker, on top of the router's allow-list.
    pub env: Vec<(String, String)>,
    /// The protocol version this harness implements.
    pub protocol_version: u32,
    /// The logical executable the harness runs under, e.g. `node` or `python`.
    pub executable: String,
}

impl WorkerHarness {
    /// Builds a harness that runs under `executable` with no extra flags.
    #[must_use]
    pub fn new(
        filename: impl Into<String>,
        source: impl Into<String>,
        executable: impl Into<String>,
    ) -> Self {
        Self {
            filename: filename.into(),
            source: source.into(),
            args_before_script: Vec::new(),
            args_after_script: Vec::new(),
            env: Vec::new(),
            protocol_version: WORKER_PROTOCOL_VERSION,
            executable: executable.into(),
        }
    }

    /// Adds an interpreter flag placed before the script path.
    #[must_use]
    pub fn with_flag(mut self, flag: impl Into<String>) -> Self {
        self.args_before_script.push(flag.into());
        self
    }

    /// Adds an environment variable the worker needs.
    #[must_use]
    pub fn with_env(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    /// The full argument vector for launching `script_path`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinyruntime_bus::WorkerHarness;
    /// let harness = WorkerHarness::new("pool_worker.js", "// ...", "node")
    ///     .with_flag("--experimental-vm-modules");
    /// assert_eq!(
    ///     harness.command_args("/cache/pool_worker.js"),
    ///     vec!["--experimental-vm-modules".to_string(), "/cache/pool_worker.js".to_string()],
    /// );
    /// ```
    #[must_use]
    pub fn command_args(&self, script_path: &str) -> Vec<String> {
        let mut args = self.args_before_script.clone();
        args.push(script_path.to_owned());
        args.extend(self.args_after_script.iter().cloned());
        args
    }
}
