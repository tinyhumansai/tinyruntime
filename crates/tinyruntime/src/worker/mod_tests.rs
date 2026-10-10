//! Caller abandonment and bounded persistent worker behavior.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures assert failures by panicking, matching existing suites"
)]
use super::*;

#[tokio::test]
async fn retired_handles_do_not_exhaust_future_reservations() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    for _ in 0..5000 {
        let handle = manager.reserve().await.unwrap();
        assert_eq!(
            manager.stop(&handle).await.kind,
            WorkerOutcomeKind::Complete
        );
        assert_eq!(manager.start(&handle).await.kind, WorkerOutcomeKind::Closed);
    }
    manager.shutdown().await;
    assert!(manager.reserve().await.is_err());
}

#[cfg(unix)]
pub(super) fn plan(source: &str) -> WorkerPlan {
    WorkerPlan {
        source: source.into(),
        command: tinyruntime_bus::worker::WorkerCommand {
            executable: "/bin/sh".into(),
            args: Vec::new(),
            env: Vec::new(),
            timeout_ms: None,
        },
        preparation: Vec::new(),
        backends: vec!["alpha".into()],
        startup_timeout_ms: 30_000,
        request_timeout_ms: 60_000,
        idle_backend: None,
        idle_timeout_ms: 0,
    }
}

#[cfg(unix)]
pub(super) const HEALTHY: &str = r#"
printf '%s\n' '{"ready":true,"protocol":1,"backends":["alpha"]}'
count=0
while IFS= read -r line; do
  count=$((count + 1))
  id=${line#*\"id\":\"}; id=${id%%\"*}
  printf '{"id":"%s","ok":true,"result":%s}\n' "$id" "$count"
done
"#;

#[cfg(unix)]
#[tokio::test]
async fn replayed_operations_do_not_repeat_worker_side_effects() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    assert_eq!(
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: plan(HEALTHY)
            })
            .await
            .kind,
        WorkerOutcomeKind::Complete
    );
    let request = WorkerRequest {
        handle: handle.clone(),
        operation: 1,
        method: "alpha.run".into(),
        params: serde_json::Value::Null,
    };
    let first = manager.request(request.clone()).await;
    assert_eq!(first.response.unwrap().result, Some(serde_json::json!(1)));
    assert_eq!(
        manager
            .request(request.clone())
            .await
            .response
            .unwrap()
            .result,
        Some(serde_json::json!(1))
    );
    let mut conflict = request.clone();
    conflict.method = "other".into();
    assert_eq!(
        manager.request(conflict).await.kind,
        WorkerOutcomeKind::Invalid
    );
    for operation in 2..20 {
        let mut next = request.clone();
        next.operation = operation;
        assert_eq!(
            manager.request(next).await.response.unwrap().result,
            Some(serde_json::json!(operation))
        );
    }
    assert_eq!(
        manager.request(request).await.kind,
        WorkerOutcomeKind::Expired
    );
    assert_eq!(
        manager.stop(&handle).await.kind,
        WorkerOutcomeKind::Complete
    );
}

#[cfg(unix)]
#[tokio::test]
async fn request_deadline_includes_pending_startup() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mut launch = plan("exec sleep 600");
    launch.request_timeout_ms = 10;
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: launch,
        })
        .await;
    let request = WorkerRequest {
        handle,
        operation: 1,
        method: "alpha.run".into(),
        params: serde_json::Value::Null,
    };
    let result = tokio::time::timeout(Duration::from_secs(1), manager.request(request)).await;
    manager.shutdown().await;
    assert!(result.is_ok(), "whole request budget must include startup");
    assert_eq!(result.unwrap().reason.as_deref(), Some("request_timeout"));
}

#[tokio::test(start_paused = true)]
async fn abandoned_idle_reservations_expire_and_release_capacity() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let mut old = Vec::new();
    for _ in 0..CAPACITY {
        old.push(manager.reserve().await.unwrap());
    }
    assert_eq!(
        manager.reserve().await.unwrap_err().kind,
        WorkerOutcomeKind::Busy
    );
    tokio::time::advance(RESERVATION_LIFETIME + Duration::from_secs(1)).await;
    let handle = manager.reserve().await.unwrap();
    for old_handle in old {
        assert_eq!(
            manager.start(&old_handle).await.kind,
            WorkerOutcomeKind::Closed
        );
    }
    assert_eq!(
        manager.stop(&handle).await.kind,
        WorkerOutcomeKind::Complete
    );
}

#[cfg(all(unix, target_os = "linux"))]
async fn await_pid(path: &std::path::Path) -> u32 {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(value) = tokio::fs::read_to_string(path).await
                && let Ok(pid) = value.trim().parse()
            {
                return pid;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[cfg(all(unix, target_os = "linux"))]
fn exited(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).map_or(true, |stat| {
        stat.split(')')
            .nth(1)
            .unwrap()
            .trim_start()
            .starts_with('Z')
    })
}

#[cfg(all(unix, target_os = "linux"))]
#[tokio::test]
async fn dropped_start_waiter_and_shutdown_still_kill_and_reap_native_startup() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mark = root.path().join("pid");
    let mut launch = plan("echo $$ > \"$MARK\"; exec sleep 600");
    launch
        .command
        .env
        .push(("MARK".into(), mark.display().to_string()));
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: launch,
        })
        .await;
    let task_manager = manager.clone();
    let waiter = tokio::spawn(async move { task_manager.start(&handle).await });
    let pid = await_pid(&mark).await;
    waiter.abort();
    let _ = waiter.await;
    manager.shutdown().await;
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "direct child must be reaped before shutdown reply"
    );
    assert!(manager.reserve().await.is_err());
}

#[cfg(all(unix, target_os = "linux"))]
#[tokio::test]
async fn canceled_prepare_waiter_and_shutdown_wait_for_preparation_native_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mark = root.path().join("pid");
    let mut launch = plan(HEALTHY);
    launch
        .preparation
        .push(tinyruntime_bus::worker::WorkerCommand {
            executable: "/bin/sh".into(),
            args: vec!["-c".into(), "echo $$ > \"$MARK\"; exec sleep 600".into()],
            env: vec![("MARK".into(), mark.display().to_string())],
            timeout_ms: None,
        });
    let task_manager = manager.clone();
    let waiter = tokio::spawn(async move {
        task_manager
            .prepare(WorkerPrepare {
                handle,
                plan: launch,
            })
            .await
    });
    let pid = await_pid(&mark).await;
    waiter.abort();
    let _ = waiter.await;
    manager.shutdown().await;
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(manager.book.lock().await.slots.is_empty());
}

#[cfg(all(unix, target_os = "linux"))]
#[tokio::test]
async fn stop_kills_worker_descendants_before_reply() {
    nix::sys::prctl::set_child_subreaper(true).unwrap();
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mark = root.path().join("pid");
    let mut launch = plan(
        "sleep 600 & echo $! > \"$MARK\"; printf '%s\\n' '{\"ready\":true,\"protocol\":1}'; wait",
    );
    launch
        .command
        .env
        .push(("MARK".into(), mark.display().to_string()));
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: launch,
        })
        .await;
    assert_eq!(
        manager.start(&handle).await.kind,
        WorkerOutcomeKind::Complete
    );
    let pid = await_pid(&mark).await;
    assert!(!exited(pid));
    assert_eq!(
        manager.stop(&handle).await.kind,
        WorkerOutcomeKind::Complete
    );
    assert!(
        exited(pid),
        "descendant must have terminated before stop reply"
    );
    assert!(
        !std::path::Path::new(&format!("/proc/{pid}")).exists(),
        "fixture descendant must be reaped too"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cleanup_fault_retains_native_ownership_for_public_stop_retry() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: plan(HEALTHY),
        })
        .await;
    manager.start(&handle).await;
    let (reply, response) = oneshot::channel();
    manager.book.lock().await.slots[&handle]
        .commands
        .as_ref()
        .unwrap()
        .send(Command::FailReap(reply))
        .await
        .unwrap();
    response.await.unwrap();
    assert_eq!(manager.stop(&handle).await.kind, WorkerOutcomeKind::Failed);
    assert_eq!(
        manager.book.lock().await.slots.len(),
        1,
        "native ownership must remain retryable"
    );
    assert_eq!(
        manager.stop(&handle).await.kind,
        WorkerOutcomeKind::Complete
    );
    assert!(manager.book.lock().await.slots.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn failed_commands_and_requests_do_not_poison_reclaimed_admission() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    for _ in 0..CAPACITY * 2 {
        let handle = manager.reserve().await.unwrap();
        let mut launch = plan(HEALTHY);
        launch.command.executable = "/definitely-not-an-existing-runtime".into();
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: launch,
            })
            .await;
        assert_eq!(manager.start(&handle).await.kind, WorkerOutcomeKind::Failed);
        assert_eq!(
            manager.stop(&handle).await.kind,
            WorkerOutcomeKind::Complete
        );
        let handle = manager.reserve().await.unwrap();
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: plan("printf '%s\\n' '{\"ready\":true,\"protocol\":1}'; exit 0"),
            })
            .await;
        let result = manager
            .request(WorkerRequest {
                handle: handle.clone(),
                operation: 1,
                method: "alpha.run".into(),
                params: serde_json::Value::Null,
            })
            .await;
        assert_eq!(result.kind, WorkerOutcomeKind::Failed);
        assert_eq!(
            manager.stop(&handle).await.kind,
            WorkerOutcomeKind::Complete
        );
    }
    assert!(manager.book.lock().await.slots.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn invalid_plans_and_requests_never_begin_native_work() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let unknown = WorkerHandle("foreign:1".into());
    assert_eq!(
        manager.start(&unknown).await.kind,
        WorkerOutcomeKind::Unknown
    );
    assert_eq!(
        manager.stop(&unknown).await.kind,
        WorkerOutcomeKind::Unknown
    );
    assert_eq!(
        manager.status(&unknown).await.unwrap_err().kind,
        WorkerOutcomeKind::Unknown
    );
    let handle = manager.reserve().await.unwrap();
    assert_eq!(
        manager.start(&handle).await.kind,
        WorkerOutcomeKind::Invalid
    );
    let mut invalid = plan(HEALTHY);
    invalid.command.executable = "bad\0command".into();
    assert_eq!(
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: invalid
            })
            .await
            .kind,
        WorkerOutcomeKind::Invalid
    );
    assert!(!root.path().read_dir().unwrap().any(|_| true));
    let valid = plan(HEALTHY);
    assert_eq!(
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: valid.clone()
            })
            .await
            .kind,
        WorkerOutcomeKind::Complete
    );
    assert_eq!(
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: valid
            })
            .await
            .kind,
        WorkerOutcomeKind::Complete
    );
    assert_eq!(
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: plan("different script")
            })
            .await
            .kind,
        WorkerOutcomeKind::Invalid
    );
    for (operation, params) in [
        (0, serde_json::Value::Null),
        (1, serde_json::json!("x".repeat(native::MAX_LINE))),
    ] {
        assert_eq!(
            manager
                .request(WorkerRequest {
                    handle: handle.clone(),
                    operation,
                    method: "alpha.run".into(),
                    params
                })
                .await
                .kind,
            WorkerOutcomeKind::Invalid
        );
    }
    manager.shutdown().await;
    assert_eq!(
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: plan(HEALTHY)
            })
            .await
            .kind,
        WorkerOutcomeKind::Closed
    );
    assert_eq!(manager.start(&handle).await.kind, WorkerOutcomeKind::Closed);
}

#[cfg(unix)]
#[tokio::test]
async fn malformed_startup_and_oversized_response_fail_without_sensitive_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    for source in [
        "printf '%s\\n' 'private malformed payload'",
        "printf '%s\\n' '{\"ready\":false,\"error\":\"private credential\"}'",
        "printf '%s\\n' '{\"ready\":true,\"protocol\":2}'",
    ] {
        let handle = manager.reserve().await.unwrap();
        manager
            .prepare(WorkerPrepare {
                handle: handle.clone(),
                plan: plan(source),
            })
            .await;
        let failed = manager.start(&handle).await;
        assert_eq!(failed.kind, WorkerOutcomeKind::Failed);
        assert!(!failed.reason.unwrap().contains("private"));
        assert_eq!(
            manager.start(&handle).await.reason.as_deref(),
            Some("startup_backoff")
        );
        manager.stop(&handle).await;
    }
    let handle = manager.reserve().await.unwrap();
    manager.prepare(WorkerPrepare { handle: handle.clone(), plan: plan("printf '%s\\n' '{\"ready\":true,\"protocol\":1}'; read line; head -c 300000 /dev/zero | tr '\\000' x") }).await;
    assert_eq!(
        manager
            .request(WorkerRequest {
                handle: handle.clone(),
                operation: 1,
                method: "alpha.run".into(),
                params: serde_json::Value::Null
            })
            .await
            .reason
            .as_deref(),
        Some("frame_limit")
    );
    manager.stop(&handle).await;
}

#[cfg(unix)]
#[tokio::test]
async fn explicit_preparation_steps_drain_both_pipes_and_fail_bounded_overflow() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    for (script, expected) in [
        ("printf a; printf b >&2", WorkerOutcomeKind::Complete),
        ("exit 7", WorkerOutcomeKind::Failed),
        ("head -c 1200000 /dev/zero >&2", WorkerOutcomeKind::Failed),
    ] {
        let handle = manager.reserve().await.unwrap();
        let mut launch = plan(HEALTHY);
        launch
            .preparation
            .push(tinyruntime_bus::worker::WorkerCommand {
                executable: "/bin/sh".into(),
                args: vec!["-c".into(), script.into()],
                env: Vec::new(),
                timeout_ms: None,
            });
        assert_eq!(
            manager
                .prepare(WorkerPrepare {
                    handle: handle.clone(),
                    plan: launch
                })
                .await
                .kind,
            expected
        );
        assert_eq!(
            manager.stop(&handle).await.kind,
            WorkerOutcomeKind::Complete
        );
    }
    assert!(manager.book.lock().await.slots.is_empty());
}

#[cfg(all(unix, target_os = "linux"))]
#[tokio::test]
async fn dropping_the_last_manager_signals_and_reaps_native_ownership() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mark = root.path().join("pid");
    let mut launch = plan(&format!("echo $$ > \"$MARK\"; {HEALTHY}"));
    launch
        .command
        .env
        .push(("MARK".into(), mark.display().to_string()));
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: launch,
        })
        .await;
    manager.start(&handle).await;
    let pid = await_pid(&mark).await;
    let mut state = manager.book.lock().await.slots[&handle].state.subscribe();
    drop(manager);
    assert!(wait_closed(&mut state).await);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
}

#[cfg(unix)]
#[tokio::test]
async fn total_junk_response_output_is_bounded_even_when_each_frame_fits() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let source = "printf '%s\\n' '{\"ready\":true,\"protocol\":1}'; read line; for n in 1 2 3 4 5 6; do head -c 200000 /dev/zero | tr '\\000' x; printf '\\n'; done";
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: plan(source),
        })
        .await;
    let result = manager
        .request(WorkerRequest {
            handle: handle.clone(),
            operation: 1,
            method: "alpha.run".into(),
            params: serde_json::Value::Null,
        })
        .await;
    assert_eq!(result.reason.as_deref(), Some("output_limit"));
    manager.stop(&handle).await;
}

#[cfg(unix)]
#[tokio::test]
async fn successful_request_idle_expiry_restarts_and_preserves_backend_order() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mut launch = plan(HEALTHY);
    launch.backends = vec!["beta".into(), "alpha".into()];
    launch.idle_backend = Some("alpha".into());
    launch.idle_timeout_ms = 100;
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: launch,
        })
        .await;
    let request = WorkerRequest {
        handle: handle.clone(),
        operation: 1,
        method: "alpha.run".into(),
        params: serde_json::Value::Null,
    };
    assert_eq!(
        manager
            .request(request.clone())
            .await
            .response
            .unwrap()
            .result,
        Some(serde_json::json!(1))
    );
    let status = manager.status(&handle).await.unwrap();
    assert_eq!(status.server.backends[0].id, "beta");
    assert!(!status.server.backends[0].ready);
    assert_eq!(status.server.backends[1].id, "alpha");
    assert!(status.server.backends[1].ready);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::resume();
    let mut next = request;
    next.operation = 2;
    assert_eq!(
        manager.request(next).await.response.unwrap().result,
        Some(serde_json::json!(1)),
        "idle worker must be rebuilt, not reused"
    );
    manager.stop(&handle).await;
}

#[cfg(unix)]
#[tokio::test]
async fn failed_request_restarts_once_without_translating_the_jsonl_method() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mark = root.path().join("first-worker");
    let mut launch = plan(&format!(
        "if [ ! -f \"$MARK\" ]; then touch \"$MARK\"; printf '%s\\n' '{{\"ready\":true,\"protocol\":1}}'; read line; exit 1; fi\n{HEALTHY}"
    ));
    launch
        .command
        .env
        .push(("MARK".into(), mark.display().to_string()));
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: launch,
        })
        .await;
    let request = WorkerRequest {
        handle: handle.clone(),
        operation: 1,
        method: "alpha.run".into(),
        params: serde_json::json!({"payload":"fixture"}),
    };
    assert_eq!(
        manager.request(request).await.response.unwrap().result,
        Some(serde_json::json!(1))
    );
    manager.stop(&handle).await;
}

#[cfg(unix)]
#[tokio::test]
async fn request_deadline_during_idle_cleanup_keeps_stop_and_shutdown_waiting() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().to_path_buf());
    let handle = manager.reserve().await.unwrap();
    let mut launch = plan(HEALTHY);
    launch.idle_backend = Some("alpha".into());
    launch.idle_timeout_ms = 100;
    launch.request_timeout_ms = 1000;
    manager
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: launch,
        })
        .await;
    assert_eq!(
        manager.start(&handle).await.kind,
        WorkerOutcomeKind::Complete
    );
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let (reply, response) = oneshot::channel();
    manager.book.lock().await.slots[&handle]
        .commands
        .as_ref()
        .unwrap()
        .send(Command::GateReap(entered.clone(), release.clone(), reply))
        .await
        .unwrap();
    response.await.unwrap();
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(1)).await;
    let pending = {
        let manager = manager.clone();
        let handle = handle.clone();
        tokio::spawn(async move {
            manager
                .request(WorkerRequest {
                    handle,
                    operation: 1,
                    method: "alpha.run".into(),
                    params: serde_json::Value::Null,
                })
                .await
        })
    };
    entered.notified().await;
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(
        pending.await.unwrap().reason.as_deref(),
        Some("request_timeout")
    );
    let stop_ready = tokio::time::timeout(Duration::from_secs(1), manager.stop(&handle))
        .await
        .is_ok();
    let shutdown_ready = tokio::time::timeout(Duration::from_secs(1), manager.shutdown())
        .await
        .is_ok();
    release.notify_one();
    assert_eq!(manager.shutdown().await.kind, WorkerOutcomeKind::Complete);
    assert!(
        !stop_ready,
        "Stop must await the native cleanup canceled by Request"
    );
    assert!(
        !shutdown_ready,
        "Shutdown must await that same outstanding native cleanup"
    );
    assert!(manager.book.lock().await.slots.is_empty());
}

#[cfg(unix)]
#[tokio::test]
async fn unapproved_cache_scope_is_rejected_before_creating_any_cache_data() {
    let root = tempfile::tempdir().unwrap();
    let manager = WorkerManager::new(root.path().join("workers"));
    let handle = manager.reserve().await.unwrap();
    let result = manager
        .prepare_cached(tinyruntime_bus::worker::WorkerPrepareCached {
            handle: handle.clone(),
            plan: plan(HEALTHY),
            recipe: tinyruntime_bus::worker::CacheRecipe {
                scope: "unapproved".into(),
                artifacts: Vec::new(),
                steps: Vec::new(),
                required: Vec::new(),
                marker: tinyruntime_bus::worker::CacheArtifact {
                    path: "ready".into(),
                    bytes: b"v1".to_vec(),
                },
                adoption: tinyruntime_bus::worker::CacheAdoptionPolicy::Strict,
                timeout_ms: 1000,
            },
        })
        .await;
    assert_eq!(result.kind, WorkerOutcomeKind::Invalid);
    assert!(!root.path().join("workers").exists());
    manager.stop(&handle).await;
}
