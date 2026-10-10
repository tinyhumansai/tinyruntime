//! Bounded framing fixtures without network or external interpreter dependencies.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures assert failures by panicking, matching existing suites"
)]
use super::*;
use std::future::Future;

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

#[test]
fn a_step_deadline_is_bounded_by_the_enclosing_operation_deadline() {
    let command = WorkerCommand {
        executable: "/bin/sh".into(),
        args: Vec::new(),
        env: Vec::new(),
        timeout_ms: Some(20),
    };
    let whole = Instant::now() + std::time::Duration::from_secs(10);
    let step = command_deadline(&command, whole);
    assert!(step < whole);
    assert_eq!(command_deadline(&command, step), step);
    let longer_step = WorkerCommand {
        timeout_ms: Some(20_000),
        ..command.clone()
    };
    assert_eq!(command_deadline(&longer_step, whole), whole);
    let no_step_limit = WorkerCommand {
        timeout_ms: None,
        ..command
    };
    assert_eq!(command_deadline(&no_step_limit, whole), whole);
}

#[cfg(target_os = "linux")]
#[tokio::test(start_paused = true)]
async fn a_step_timeout_kills_and_joins_its_native_process_group() {
    let root = tempfile::tempdir().unwrap();
    let command = WorkerCommand {
        executable: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 600".into()],
        env: Vec::new(),
        timeout_ms: Some(30),
    };
    let mut preparation = Preparation::spawn(&command, root.path()).unwrap();
    let pid = preparation.child.as_ref().unwrap().id().unwrap();
    let (_stop, mut stop) = watch::channel(false);
    let deadline = command_deadline(
        &command,
        Instant::now() + std::time::Duration::from_secs(50),
    );
    let result = {
        let execute = preparation.execute(&mut stop, deadline);
        tokio::pin!(execute);
        assert!(
            std::future::poll_fn(|cx| {
                std::task::Poll::Ready(execute.as_mut().poll(cx).is_pending())
            })
            .await
        );
        tokio::time::advance(std::time::Duration::from_millis(31)).await;
        execute.await
    };
    assert_eq!(result, Err("prepare_timeout"));
    preparation.cleanup().await.unwrap();
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
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
        timeout_ms: None,
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
        timeout_ms: None,
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

#[cfg(target_os = "linux")]
#[tokio::test]
async fn canceled_cleanup_wait_retains_reaping_until_a_later_cleanup_finishes() {
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
        timeout_ms: None,
    };
    let mut process = Process::spawn(&command, root.path(), &script).unwrap();
    let pid = process.child.as_ref().unwrap().id().unwrap();
    process.handshake().await.unwrap();
    // Isolate native reaping from the separately tested stderr join.
    if let Some(task) = process.stderr.take() {
        task.abort();
        let _ = task.await;
    }
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    process.reap_gate = Some((
        std::sync::Arc::new(tokio::sync::Notify::new()),
        release.clone(),
    ));
    {
        let cleanup = process.cleanup();
        tokio::pin!(cleanup);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                cleanup.as_mut().poll(cx).is_pending()
            ))
            .await
        );
    }
    let prematurely_complete = {
        let cleanup = process.cleanup();
        tokio::pin!(cleanup);
        std::future::poll_fn(|cx| std::task::Poll::Ready(cleanup.as_mut().poll(cx).is_ready()))
            .await
    };
    release.notify_one();
    process.cleanup().await.unwrap();
    await_reaped(pid).await;
    assert!(
        !prematurely_complete,
        "canceling a cleanup must not discard the original native wait"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn canceled_preparation_cleanup_keeps_the_original_group_wait_owned() {
    let root = tempfile::tempdir().unwrap();
    let command = WorkerCommand {
        executable: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 600".into()],
        env: Vec::new(),
        timeout_ms: None,
    };
    let mut process = Preparation::spawn(&command, root.path()).unwrap();
    let pid = process.child.as_ref().unwrap().id().unwrap();
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    process.reap_gate = Some((entered.clone(), release.clone()));
    {
        let cleanup = process.cleanup();
        tokio::pin!(cleanup);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                cleanup.as_mut().poll(cx).is_pending()
            ))
            .await
        );
    }
    entered.notified().await;
    {
        let cleanup = process.cleanup();
        tokio::pin!(cleanup);
        assert!(
            std::future::poll_fn(|cx| std::task::Poll::Ready(
                cleanup.as_mut().poll(cx).is_pending()
            ))
            .await
        );
    }
    release.notify_one();
    process.cleanup().await.unwrap();
    await_reaped(pid).await;
}
