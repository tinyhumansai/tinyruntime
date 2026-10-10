//! Bounded framing fixtures without network or external interpreter dependencies.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures assert failures by panicking, matching existing suites"
)]
use super::*;

#[tokio::test]
async fn rejects_a_line_larger_than_the_memory_bound() {
    let input = vec![b'x'; MAX_LINE + 1];
    let mut reader = BufReader::new(input.as_slice());
    assert_eq!(line(&mut reader).await.unwrap_err(), "frame_limit");
}

#[tokio::test]
async fn preserves_newline_frames_and_rejects_truncated_eof() {
    let mut reader = BufReader::new(b"first\nsecond\ntruncated".as_slice());
    assert_eq!(line(&mut reader).await.unwrap(), b"first\n");
    assert_eq!(line(&mut reader).await.unwrap(), b"second\n");
    assert_eq!(line(&mut reader).await.unwrap_err(), "worker_closed");
}

#[cfg(target_os = "linux")]
async fn await_reaped(pid: u32) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while std::path::Path::new(&format!("/proc/{pid}")).exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn dropping_native_ownership_signals_and_awaits_reaping_on_the_live_runtime() {
    let root = tempfile::tempdir().unwrap();
    let script = root.path().join("worker");
    tokio::fs::write(
        &script,
        "printf '%s\\n' '{\"ready\":true,\"protocol\":1}'; exec sleep 600",
    )
    .await
    .unwrap();
    let command = WorkerCommand {
        executable: "/bin/sh".into(),
        args: Vec::new(),
        env: Vec::new(),
    };
    let mut process = Process::spawn(&command, root.path(), &script).unwrap();
    let pid = process.child.as_ref().unwrap().id().unwrap();
    process.handshake().await.unwrap();
    drop(process);
    await_reaped(pid).await;
    let command = WorkerCommand {
        executable: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 600".into()],
        env: Vec::new(),
    };
    let preparation = Preparation::spawn(&command, root.path()).unwrap();
    let pid = preparation.child.as_ref().unwrap().id().unwrap();
    drop(preparation);
    await_reaped(pid).await;
}

#[tokio::test]
async fn closed_cancellation_channel_is_a_stop_signal() {
    let (stop, mut receiver) = watch::channel(false);
    drop(stop);
    canceled(&mut receiver).await;
}
