//! Bounded JSONL I/O and owned native process groups with explicit reaping.
use std::path::Path;
use std::process::Stdio;

use command_group::{AsyncCommandGroup, AsyncGroupChild};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{ChildStderr, ChildStdin, ChildStdout, Command};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use tinyruntime_bus::worker::{
    PROTOCOL_VERSION, ReadyLine, ServerRequest, ServerResponse, WorkerCommand,
};

pub(super) const MAX_LINE: usize = 256 * 1024;
pub(super) const MAX_OPERATION_BYTES: usize = 1024 * 1024;

/// A frame reader that rejects expansion before extending its owned buffer.
async fn line<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    loop {
        let chunk = reader.fill_buf().await.map_err(|_| "read_failed")?;
        if chunk.is_empty() {
            return Err("worker_closed");
        }
        let count = chunk
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(chunk.len(), |n| n + 1);
        if count > MAX_LINE.saturating_sub(bytes.len()) {
            return Err("frame_limit");
        }
        bytes.extend_from_slice(&chunk[..count]);
        let complete = bytes.last() == Some(&b'\n');
        reader.consume(count);
        if complete {
            return Ok(bytes);
        }
    }
}

fn command(plan: &WorkerCommand, root: &Path) -> Command {
    let mut cmd = Command::new(&plan.executable);
    cmd.args(&plan.args).current_dir(root).env_clear();
    let bin = Path::new(&plan.executable)
        .parent()
        .unwrap_or_else(|| Path::new("."));
    cmd.envs(crate::pool::env::build(bin, &plan.env));
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd
}

/// Process ownership survives cancellation of handshake/request futures.
#[derive(Debug)]
pub(super) struct Process {
    child: Option<AsyncGroupChild>,
    #[cfg(test)]
    pub(super) reap_failures: usize,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Option<JoinHandle<()>>,
}

impl Process {
    pub(super) fn spawn(
        plan: &WorkerCommand,
        root: &Path,
        script: &Path,
    ) -> Result<Self, &'static str> {
        let mut cmd = command(plan, root);
        cmd.arg(script);
        let mut child = cmd
            .group()
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "spawn_failed")?;
        // Piped stdio is guaranteed by our command construction.
        let stdin = child.inner().stdin.take().ok_or("stdin_missing")?;
        let stdout = child.inner().stdout.take().ok_or("stdout_missing")?;
        let stderr = child.inner().stderr.take().map(|mut stream| {
            tokio::spawn(async move {
                let mut buffer = Box::new([0_u8; 8192]);
                // No payload logging and no line aggregation on stderr.
                while let Ok(count) = stream.read(&mut *buffer).await {
                    if count == 0 {
                        break;
                    }
                }
            })
        });
        Ok(Self {
            child: Some(child),
            #[cfg(test)]
            reap_failures: 0,
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
        })
    }

    pub(super) async fn handshake(&mut self) -> Result<ReadyLine, &'static str> {
        let ready: ReadyLine =
            serde_json::from_slice(&line(&mut self.stdout).await?).map_err(|_| "invalid_ready")?;
        if !ready.ready {
            return Err("worker_not_ready");
        }
        if ready.protocol != Some(PROTOCOL_VERSION) {
            return Err("protocol_mismatch");
        }
        Ok(ready)
    }

    pub(super) async fn request(
        &mut self,
        request: &ServerRequest,
    ) -> Result<ServerResponse, &'static str> {
        let mut bytes = serde_json::to_vec(request).map_err(|_| "encode_failed")?;
        if bytes.len() >= MAX_LINE {
            return Err("request_limit");
        }
        bytes.push(b'\n');
        self.stdin
            .write_all(&bytes)
            .await
            .map_err(|_| "write_failed")?;
        self.stdin.flush().await.map_err(|_| "write_failed")?;
        let mut remaining = MAX_OPERATION_BYTES;
        loop {
            let bytes = line(&mut self.stdout).await?;
            if bytes.len() > remaining {
                return Err("output_limit");
            }
            remaining -= bytes.len();
            if let Ok(response) = serde_json::from_slice::<ServerResponse>(&bytes)
                && response.id.as_deref() == Some(request.id.as_str())
            {
                return Ok(response);
            }
        }
    }

    /// Kills the entire native group/job, reaps, then joins pipe ownership.
    pub(super) async fn cleanup(&mut self) -> Result<(), &'static str> {
        if let Some(mut child) = self.child.take() {
            // Already exited groups can reject the signal; wait remains required.
            let _ = child.start_kill();
            #[cfg(test)]
            if self.reap_failures > 0 {
                self.reap_failures -= 1;
                self.child = Some(child);
                return Err("reap_failed");
            }
            if child.wait().await.is_err() {
                self.child = Some(child);
                return Err("reap_failed");
            }
        }
        if let Some(task) = self.stderr.take() {
            task.abort();
            let _ = task.await;
        }
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            let stderr = self.stderr.take();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                    if let Some(task) = stderr {
                        task.abort();
                        let _ = task.await;
                    }
                });
            } else if let Some(task) = stderr {
                task.abort();
            }
        }
    }
}

/// Preparation retains native ownership even when wait/reap fails.
#[derive(Debug)]
pub(super) struct Preparation {
    child: Option<AsyncGroupChild>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
}

impl Preparation {
    pub(super) fn spawn(plan: &WorkerCommand, root: &Path) -> Result<Self, &'static str> {
        let mut cmd = command(plan, root);
        cmd.stdin(Stdio::null());
        let mut child = cmd
            .group()
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "prepare_spawn_failed")?;
        let stdout = child.inner().stdout.take();
        let stderr = child.inner().stderr.take();
        Ok(Self {
            child: Some(child),
            stdout,
            stderr,
        })
    }

    pub(super) async fn execute(
        &mut self,
        stop: &mut watch::Receiver<bool>,
        deadline: Instant,
    ) -> Result<(), &'static str> {
        let stdout = self.stdout.take().ok_or("stdout_missing")?;
        let stderr = self.stderr.take().ok_or("stderr_missing")?;
        let count = AtomicUsize::new(0);
        let read = async {
            tokio::try_join!(drain_bounded(stdout, &count), drain_bounded(stderr, &count))?;
            Ok(())
        };
        let result = tokio::select! {
            biased;
            () = canceled(stop) => Err("canceled"),
            result = tokio::time::timeout_at(deadline, read) => result.unwrap_or(Err("prepare_timeout")),
        };
        let child = self.child.as_mut().ok_or("prepare_closed")?;
        if result.is_err() {
            let _ = child.start_kill();
        }
        let wait = tokio::select! {
            biased;
            () = canceled(stop) => Err("canceled"),
            result = tokio::time::timeout_at(deadline, child.wait()) =>
                result.map_err(|_| "prepare_timeout").and_then(|s| s.map_err(|_| "prepare_reap_failed")),
        };
        if wait.is_err() {
            self.cleanup().await?;
        }
        result?;
        if wait?.success() {
            Ok(())
        } else {
            Err("prepare_failed")
        }
    }

    pub(super) async fn cleanup(&mut self) -> Result<(), &'static str> {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
            child.wait().await.map_err(|_| "prepare_reap_failed")?;
        }
        self.child = None;
        self.stdout = None;
        self.stderr = None;
        Ok(())
    }
}

impl Drop for Preparation {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = child.wait().await;
                });
            }
        }
    }
}

async fn drain_bounded<R: tokio::io::AsyncRead + Unpin>(
    mut pipe: R,
    total: &AtomicUsize,
) -> Result<(), &'static str> {
    let mut buffer = Box::new([0_u8; 8192]);
    loop {
        let count = pipe
            .read(&mut *buffer)
            .await
            .map_err(|_| "prepare_read_failed")?;
        if count == 0 {
            return Ok(());
        }
        if total.fetch_add(count, Ordering::Relaxed) + count > MAX_OPERATION_BYTES {
            return Err("prepare_output_limit");
        }
    }
}

pub(super) async fn canceled(stop: &mut watch::Receiver<bool>) {
    loop {
        if *stop.borrow_and_update() {
            return;
        }
        if stop.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
