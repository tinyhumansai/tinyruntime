//! What a host can learn about the worker without sending it a request.

use serde::{Deserialize, Serialize};

/// One backend's state inside the worker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendStatus {
    /// The backend id, such as `spacy`.
    pub id: String,
    /// Whether the host asked for this backend.
    pub enabled: bool,
    /// Whether the worker reported it loaded in its handshake.
    pub ready: bool,
    /// Extra detail, when there is any.
    pub message: Option<String>,
}

/// The worker as a whole.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerStatus {
    /// Whether a worker is configured at all.
    pub enabled: bool,
    /// Whether a child process is alive right now.
    pub running: bool,
    /// Per-backend state.
    pub backends: Vec<BackendStatus>,
    /// Why the worker is disabled or unavailable, when it is.
    pub message: Option<String>,
}

impl ServerStatus {
    /// A status for a worker that is not configured or has not started.
    #[must_use]
    pub fn disabled(message: impl Into<String>) -> Self {
        Self {
            enabled: false,
            running: false,
            backends: Vec::new(),
            message: Some(message.into()),
        }
    }
}
