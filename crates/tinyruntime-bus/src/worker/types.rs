//! Declarative worker management requests; execution belongs to the router.
use super::{ServerResponse, ServerStatus};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A process-local opaque identity returned before any side effect begins.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkerHandle(pub String);

/// A complete command, supplied by a trusted recipe owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCommand {
    /// Explicit executable; no fallback is attempted.
    pub executable: String,
    /// Arguments before the module-installed script (for the final worker).
    pub args: Vec<String>,
    /// Extra environment over the router's safe allowlist.
    pub env: Vec<(String, String)>,
}

/// A bounded installation/launch description without language algorithms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerPlan {
    /// Script contents installed by the module in its owned directory.
    pub source: String,
    /// Native worker command; module appends the installed script path.
    pub command: WorkerCommand,
    /// Explicit preparation steps executed in the owned working directory.
    pub preparation: Vec<WorkerCommand>,
    /// Requested backend ids in stable order.
    pub backends: Vec<String>,
    /// Maximum whole preparation/startup duration in milliseconds.
    pub startup_timeout_ms: u64,
    /// Maximum whole request duration in milliseconds.
    pub request_timeout_ms: u64,
    /// Backend whose successful-request idle age triggers a restart.
    pub idle_backend: Option<String>,
    /// Idle interval in milliseconds; zero disables idle rebuilding.
    pub idle_timeout_ms: u64,
}

/// Preparing a known reservation; identical retries return the same outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerPrepare {
    /// Known reservation to own all preparation side effects.
    pub handle: WorkerHandle,
    /// Immutable preparation and launch plan.
    pub plan: WorkerPlan,
}

/// One caller-known operation; numbers are monotonic per resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRequest {
    /// Known live worker.
    pub handle: WorkerHandle,
    /// Positive identity; a retry must retain the same method and parameters.
    pub operation: u64,
    /// Backend-namespaced method, carried unchanged onto JSONL.
    pub method: String,
    /// Parameters, carried unchanged onto JSONL.
    pub params: Value,
}

/// Whether an operation completed, failed, or can no longer be replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerOutcomeKind {
    /// Preparation/start/stop completed.
    Complete,
    /// Worker replied to a request (including a remote failure).
    Response,
    /// Handle is retired; no side effect is repeated.
    Closed,
    /// Busy queue or live-resource capacity; safe to retry.
    Busy,
    /// Invalid plan, request, or conflicting identity.
    Invalid,
    /// Resource/operation does not belong to this manager.
    Unknown,
    /// Outcome left the bounded replay window; never reexecuted.
    Expired,
    /// Native preparation/start/protocol/request failure.
    Failed,
}

/// A bounded terminal outcome without sensitive native diagnostics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerOutcome {
    /// Classification for caller policy.
    pub kind: WorkerOutcomeKind,
    /// JSONL reply when available, preserving its original representation.
    pub response: Option<ServerResponse>,
    /// Content-free machine reason; never stdout/stderr/paths/environment.
    pub reason: Option<String>,
}

/// Manager-visible phase alongside the preserved server status vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerPhase {
    /// An idle reservation, not yet owning a native resource.
    Reserved,
    /// Module preparation executing.
    Preparing,
    /// Prepared and available to start.
    Prepared,
    /// Native handshake pending.
    Starting,
    /// Worker serving requests.
    Running,
    /// Startup failed and its five-minute backoff is active.
    Failed,
    /// Cleanup completed.
    Closed,
}

/// Worker phase and backward-compatible backend status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerStatus {
    /// Resource lifecycle phase.
    pub phase: WorkerPhase,
    /// Preserved legacy status fields.
    pub server: ServerStatus,
}
