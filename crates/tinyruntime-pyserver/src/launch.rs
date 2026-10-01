//! Everything a host decides before a worker is started.

use std::path::PathBuf;
use std::time::Duration;

/// How long a freshly spawned worker has to print its ready line.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Ceiling for a single backend request.
///
/// A fast backend answers in well under a hundred milliseconds; a model-backed
/// one can take seconds on a CPU. This is a maximum, not added latency for the
/// fast methods, so it is sized for the slow one.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// A description of one worker process: what to run and how to talk to it.
///
/// The host resolves the interpreter (provisioning a virtual environment if it
/// needs one), decides the environment, and hands this over; this crate never
/// looks at the host's configuration.
#[derive(Debug, Clone)]
pub struct ServerLaunch {
    /// The interpreter binary.
    pub python_bin: PathBuf,
    /// The worker script, normally written by [`write_script`](crate::write_script).
    pub script_path: PathBuf,
    /// Ids of the backends this worker was prepared for, in a stable order.
    pub backends: Vec<String>,
    /// Extra environment for the child, on top of the inherited one.
    pub env: Vec<(String, String)>,
    /// How long the worker has to print its ready line.
    pub handshake_timeout: Duration,
    /// How long a single request may wait for its response.
    pub request_timeout: Duration,
}

impl ServerLaunch {
    /// A launch with the default timeouts.
    #[must_use]
    pub fn new(
        python_bin: PathBuf,
        script_path: PathBuf,
        backends: Vec<String>,
        env: Vec<(String, String)>,
    ) -> Self {
        Self {
            python_bin,
            script_path,
            backends,
            env,
            handshake_timeout: DEFAULT_HANDSHAKE_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        }
    }
}
