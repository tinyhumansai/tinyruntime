//! The JSON-lines wire types spoken with the worker over stdio.
//!
//! One JSON document per line. After spawning, the worker writes a
//! [`ReadyLine`] before any request is sent; from then on every request line
//! [`ServerRequest`] is answered by one [`ServerResponse`] line carrying the same
//! `id`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The protocol revision the bundled worker script speaks.
///
/// A worker that reports a different one in its [`ReadyLine`] is refused, so a
/// stale script left on disk can never be talked to as though it were current.
pub const PROTOCOL_VERSION: u32 = 1;

/// The first line a worker writes: whether it came up and which backends loaded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyLine {
    /// `true` when the worker is ready to serve requests.
    #[serde(default)]
    pub ready: bool,
    /// The protocol revision the worker implements.
    #[serde(default)]
    pub protocol: Option<u32>,
    /// Backends the worker has loaded and can serve.
    #[serde(default)]
    pub backends: Vec<String>,
    /// Why the worker could not start, when `ready` is `false`.
    #[serde(default)]
    pub error: Option<String>,
}

/// One request line sent to the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerRequest {
    /// Correlates the response; unique per request within one worker process.
    pub id: String,
    /// Backend-namespaced method, such as `spacy.extract`.
    pub method: String,
    /// Method parameters.
    #[serde(default)]
    pub params: Value,
}

/// The failure a worker reports for a request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerError {
    /// Machine-readable failure class.
    pub code: String,
    /// Human-readable detail.
    pub message: String,
}

/// One response line received from the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerResponse {
    /// The `id` of the request this answers, if the worker could tell.
    pub id: Option<String>,
    /// Whether the request succeeded.
    #[serde(default)]
    pub ok: bool,
    /// The method's result, when `ok`.
    #[serde(default)]
    pub result: Option<Value>,
    /// The failure, when not `ok`.
    #[serde(default)]
    pub error: Option<ServerError>,
}
