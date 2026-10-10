//! Bounded JSONL I/O and owned native process groups with explicit reaping.
use std::path::Path;
use std::process::Stdio;

use command_group::{AsyncCommandGroup, AsyncGroupChild};
use futures_util::FutureExt;
use std::panic::AssertUnwindSafe;
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

/// Bound a command to its own optional deadline and the enclosing operation.
pub(super) fn command_deadline(plan: &WorkerCommand, enclosing: Instant) -> Instant {
    plan.timeout_ms.map_or(enclosing, |timeout| {
        enclosing.min(Instant::now() + std::time::Duration::from_millis(timeout))
    })
}

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

/// The task owns the group throughout command-group's uncancelled native wait.
/// Keeping its handle here also retains any Windows completion-port waiter.
#[derive(Debug)]
enum Reaper {
    Waiting(JoinHandle<(AsyncGroupChild, Result<(), &'static str>)>),
    // A runtime-aborted task cannot prove cleanup. Never acknowledge it as reaped.
    Lost,
}

#[cfg(test)]
type ReapGate = (
    std::sync::Arc<tokio::sync::Notify>,
    std::sync::Arc<tokio::sync::Notify>,
);

async fn reap(
    child: &mut Option<AsyncGroupChild>,
    reaping: &mut Option<Reaper>,
    #[cfg(test)] gate: &mut Option<ReapGate>,
    #[cfg(test)] fail_wait: bool,
) -> Result<(), &'static str> {
    if reaping.is_none()
        && let Some(mut owned) = child.take()
    {
        let _ = owned.start_kill();
        #[cfg(test)]
        let gate = gate.take();
        *reaping = Some(Reaper::Waiting(tokio::spawn(async move {
            #[cfg(test)]
            if let Some((entered, release)) = gate {
                entered.notify_one();
                release.notified().await;
            }
            #[cfg(test)]
            if fail_wait {
                return (owned, Err("reap_failed"));
            }
            // Catch a native wait panic while the outer task still owns Child,
            // so callers can retry rather than losing the resource to unwinding.
            let result = AssertUnwindSafe(owned.wait()).catch_unwind().await;
            let result = match result {
                Ok(Ok(_)) => Ok(()),
                _ => Err("reap_failed"),
            };
            (owned, result)
        })));
    }
    match reaping {
        Some(Reaper::Waiting(task)) => {
            if let Ok((owned, result)) = task.await {
                *reaping = None;
                if result.is_err() {
                    *child = Some(owned);
                }
                result
            } else {
                *reaping = Some(Reaper::Lost);
                Err("reap_task_failed")
            }
        }
        Some(Reaper::Lost) => Err("reap_task_failed"),
        None => Ok(()),
    }
}

fn cleanup_on_drop(
    child: Option<AsyncGroupChild>,
    reaping: Option<Reaper>,
    stderr: Option<JoinHandle<()>>,
) {
    let child = child.map(|mut child| {
        let _ = child.start_kill();
        child
    });
    if let Some(task) = &stderr {
        task.abort();
    }
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        runtime.spawn(async move {
            if let Some(mut child) = child {
                let _ = child.wait().await;
            }
            if let Some(Reaper::Waiting(task)) = reaping {
                let _ = task.await;
            }
            if let Some(task) = stderr {
                let _ = task.await;
            }
        });
    }
}

/// Process ownership survives cancellation of handshake/request futures.
#[derive(Debug)]
pub(super) struct Process {
    child: Option<AsyncGroupChild>,
    reaping: Option<Reaper>,
    #[cfg(test)]
    pub(super) reap_failures: usize,
    #[cfg(test)]
    pub(super) reap_gate: Option<ReapGate>,
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
            reaping: None,
            #[cfg(test)]
            reap_failures: 0,
            #[cfg(test)]
            reap_gate: None,
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
        #[cfg(test)]
        let fail_wait = self.child.is_some() && self.reap_failures > 0;
        #[cfg(test)]
        if fail_wait {
            self.reap_failures -= 1;
        }
        reap(
            &mut self.child,
            &mut self.reaping,
            #[cfg(test)]
            &mut self.reap_gate,
            #[cfg(test)]
            fail_wait,
        )
        .await?;
        if let Some(task) = self.stderr.as_mut() {
            task.abort();
            let _ = task.await;
        }
        self.stderr = None;
        Ok(())
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        cleanup_on_drop(self.child.take(), self.reaping.take(), self.stderr.take());
    }
}

/// Preparation retains native ownership even when wait/reap fails.
#[derive(Debug)]
pub(super) struct Preparation {
    child: Option<AsyncGroupChild>,
    reaping: Option<Reaper>,
    #[cfg(test)]
    reap_gate: Option<ReapGate>,
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
            reaping: None,
            #[cfg(test)]
            reap_gate: None,
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
        // Tokio direct-child wait is cancellation-safe. Group/job wait is always
        // performed by the owned, uncancelled cleanup task below and by Actor.
        let wait = tokio::select! {
            biased;
            () = canceled(stop) => Err("canceled"),
            result = tokio::time::timeout_at(deadline, child.inner().wait()) =>
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
        reap(
            &mut self.child,
            &mut self.reaping,
            #[cfg(test)]
            &mut self.reap_gate,
            #[cfg(test)]
            false,
        )
        .await
        .map_err(|_| "prepare_reap_failed")?;
        self.stdout = None;
        self.stderr = None;
        Ok(())
    }
}

impl Drop for Preparation {
    fn drop(&mut self) {
        cleanup_on_drop(self.child.take(), self.reaping.take(), None);
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
