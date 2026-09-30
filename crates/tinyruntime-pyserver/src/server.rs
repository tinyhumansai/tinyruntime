//! One worker: spawn, handshake, request/response, restart-on-failure.
//!
//! The worker is spawned lazily and kept for the life of the [`PythonServer`].
//! A request that fails resets the child and is retried once on a fresh one, so
//! a transient crash costs the caller latency rather than an error. Requests are
//! serialised behind one lock: the protocol is strictly line-in, line-out, and a
//! second in-flight request would interleave its response with the first.

use std::process::Stdio;
use std::time::{Duration, Instant};

use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

use crate::error::{Error, Result};
use crate::launch::ServerLaunch;
use crate::protocol::{PROTOCOL_VERSION, ReadyLine, ServerRequest, ServerResponse};
use crate::status::{BackendStatus, ServerStatus};

/// `CREATE_NO_WINDOW`, so Windows does not flash a console for the worker.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

struct Inner {
    child: Child,
    stdin: ChildStdin,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    ready_backends: Vec<String>,
}

impl std::fmt::Debug for Inner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inner")
            .field("next_id", &self.next_id)
            .field("ready_backends", &self.ready_backends)
            .finish_non_exhaustive()
    }
}

/// A persistent Python worker serving named backends over stdio.
#[derive(Debug)]
pub struct PythonServer {
    launch: ServerLaunch,
    inner: Mutex<Option<Inner>>,
    last_used: Mutex<Instant>,
}

impl PythonServer {
    /// A server for `launch`. Nothing is spawned until [`start`](Self::start) or
    /// the first request.
    #[must_use]
    pub fn new(launch: ServerLaunch) -> Self {
        Self {
            launch,
            inner: Mutex::new(None),
            last_used: Mutex::new(Instant::now()),
        }
    }

    /// The backend ids this server was launched for.
    #[must_use]
    pub fn backends(&self) -> &[String] {
        &self.launch.backends
    }

    /// Spawn the worker and wait for its handshake, if it is not running.
    ///
    /// # Errors
    ///
    /// Returns the spawn or handshake failure.
    pub async fn start(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if guard.is_some() {
            return Ok(());
        }
        *guard = Some(spawn_inner(&self.launch).await?);
        Ok(())
    }

    /// Send one request and decode its result.
    ///
    /// On any failure the child is reset and the request is retried once on a
    /// fresh one before the error is returned.
    ///
    /// # Errors
    ///
    /// Returns the failure of the retry: a spawn or handshake problem, a
    /// transport error, a timeout, an error reported by the worker, or a result
    /// that does not decode as `T`.
    pub async fn request<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        match self.request_once(method, params.clone()).await {
            Ok(value) => {
                self.mark_used().await;
                Ok(value)
            }
            Err(err) => {
                tracing::warn!(
                    "[runtime_python_server] request failed; restarting server before retry: {err}"
                );
                self.reset().await;
                let value = self.request_once(method, params).await?;
                self.mark_used().await;
                Ok(value)
            }
        }
    }

    async fn request_once<T>(&self, method: &str, params: Value) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let mut guard = self.inner.lock().await;
        if guard.is_none() {
            *guard = Some(spawn_inner(&self.launch).await?);
        }
        let Some(inner) = guard.as_mut() else {
            return Err(Error::Closed);
        };
        let id = inner.next_id.to_string();
        inner.next_id += 1;

        let request = ServerRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        };
        let mut line = serde_json::to_string(&request).map_err(Error::Encode)?;
        line.push('\n');
        tracing::debug!("[runtime_python_server] sending request id={id} method={method}");
        inner
            .stdin
            .write_all(line.as_bytes())
            .await
            .map_err(Error::Write)?;
        inner.stdin.flush().await.map_err(Error::Flush)?;

        loop {
            let next =
                tokio::time::timeout(self.launch.request_timeout, inner.stdout.next_line()).await;
            let line = match next {
                Ok(Ok(Some(line))) => line,
                Ok(Ok(None)) => return Err(Error::Closed),
                Ok(Err(error)) => return Err(Error::Read(error)),
                Err(_) => return Err(Error::RequestTimeout),
            };
            let response: ServerResponse = match serde_json::from_str(&line) {
                Ok(response) => response,
                Err(error) => {
                    tracing::warn!(
                        "[runtime_python_server] unparseable response skipped: {error}; line_len={}",
                        line.len()
                    );
                    continue;
                }
            };
            if response.id.as_deref() != Some(id.as_str()) {
                tracing::debug!(
                    "[runtime_python_server] skipped response for different id={:?}",
                    response.id
                );
                continue;
            }
            if !response.ok {
                let message = response.error.map_or_else(
                    || "unknown python server error".to_string(),
                    |error| format!("{}: {}", error.code, error.message),
                );
                return Err(Error::Remote {
                    method: method.to_string(),
                    message,
                });
            }
            let result = response.result.unwrap_or(Value::Null);
            return serde_json::from_value(result).map_err(|source| Error::Decode {
                method: method.to_string(),
                source,
            });
        }
    }

    /// Kill the child, if any. The next request spawns a fresh one.
    pub async fn reset(&self) {
        let mut guard = self.inner.lock().await;
        if let Some(mut inner) = guard.take()
            && let Err(error) = inner.child.start_kill()
        {
            tracing::debug!("[runtime_python_server] failed to signal child shutdown: {error}");
        }
    }

    async fn mark_used(&self) {
        *self.last_used.lock().await = Instant::now();
    }

    /// Whether no request has succeeded for at least `timeout`.
    pub async fn idle_expired(&self, timeout: Duration) -> bool {
        self.last_used.lock().await.elapsed() >= timeout
    }

    /// The worker's current state, from the handshake it last completed.
    pub async fn status(&self) -> ServerStatus {
        let guard = self.inner.lock().await;
        let ready_backends = guard
            .as_ref()
            .map_or(&[][..], |inner| inner.ready_backends.as_slice());
        ServerStatus {
            enabled: true,
            running: guard.is_some(),
            backends: self
                .launch
                .backends
                .iter()
                .map(|id| BackendStatus {
                    id: id.clone(),
                    enabled: true,
                    ready: ready_backends.iter().any(|ready| ready == id),
                    message: None,
                })
                .collect(),
            message: None,
        }
    }
}

fn drain_stderr(stderr: ChildStderr) {
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr);
        let mut buf = Vec::with_capacity(1024);
        let mut line_count = 0u64;
        let mut byte_count = 0u64;

        loop {
            buf.clear();
            match reader.read_until(b'\n', &mut buf).await {
                Ok(0) => {
                    tracing::debug!(
                        "[runtime_python_server] stderr drain closed lines={line_count} bytes={byte_count}"
                    );
                    break;
                }
                Ok(n) => {
                    line_count += 1;
                    byte_count += n as u64;
                    tracing::trace!(
                        "[runtime_python_server] drained stderr line bytes={n} total_lines={line_count} total_bytes={byte_count}"
                    );
                }
                Err(error) => {
                    tracing::debug!(
                        "[runtime_python_server] stderr drain failed after lines={line_count} bytes={byte_count}: {error}"
                    );
                    break;
                }
            }
        }
    });
}

fn spawn_child(launch: &ServerLaunch) -> Result<Child> {
    let mut cmd = Command::new(&launch.python_bin);
    // Unbuffered stdio, so a line-oriented protocol does not stall behind
    // Python's output buffering.
    cmd.arg("-u").arg(&launch.script_path);
    for (key, value) in &launch.env {
        cmd.env(key, value);
    }
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd.spawn().map_err(|source| Error::Spawn {
        bin: launch.python_bin.display().to_string(),
        script: launch.script_path.display().to_string(),
        source,
    })
}

async fn spawn_inner(launch: &ServerLaunch) -> Result<Inner> {
    tracing::info!(
        "[runtime_python_server] starting server python={} script={} backends={:?}",
        launch.python_bin.display(),
        launch.script_path.display(),
        launch.backends
    );
    let mut child = spawn_child(launch)?;
    let stdin = child.stdin.take().ok_or(Error::StdinMissing)?;
    let stdout = child.stdout.take().ok_or(Error::StdoutMissing)?;
    if let Some(stderr) = child.stderr.take() {
        drain_stderr(stderr);
    }
    let mut lines = BufReader::new(stdout).lines();

    let ready_line = match tokio::time::timeout(launch.handshake_timeout, lines.next_line()).await {
        Ok(Ok(Some(line))) => line,
        Ok(Ok(None)) => return Err(Error::ExitedBeforeReady),
        Ok(Err(error)) => return Err(Error::HandshakeRead(error)),
        Err(_) => return Err(Error::HandshakeTimeout),
    };
    let ready: ReadyLine =
        serde_json::from_str(&ready_line).map_err(|source| Error::ReadyParse {
            line: ready_line.clone(),
            source,
        })?;
    if !ready.ready {
        return Err(Error::StartFailed(
            ready.error.unwrap_or_else(|| "unknown".to_string()),
        ));
    }
    if ready.protocol != Some(PROTOCOL_VERSION) {
        return Err(Error::ProtocolMismatch {
            expected: PROTOCOL_VERSION,
            got: ready.protocol,
        });
    }
    tracing::info!(
        "[runtime_python_server] server ready backends={:?}",
        ready.backends
    );

    Ok(Inner {
        child,
        stdin,
        stdout: lines,
        next_id: 0,
        ready_backends: ready.backends,
    })
}

#[cfg(test)]
#[path = "server_test.rs"]
mod tests;
