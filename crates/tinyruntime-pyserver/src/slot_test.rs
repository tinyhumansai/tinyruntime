//! Unit tests for the shared-worker policy, run against shell-script fakes.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::{IdleRule, ServerSlot};
use crate::error::Error;
use crate::testing::{healthy, launch};

fn names(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_string()).collect()
}

#[tokio::test]
async fn an_empty_slot_reports_it_has_not_started() {
    let slot = ServerSlot::new();
    let status = slot.status().await;
    assert!(!status.enabled && !status.running);
    assert_eq!(
        status.message.as_deref(),
        Some("runtime python server has not started")
    );
}

#[tokio::test]
async fn one_worker_is_built_once_and_shared() {
    let dir = tempfile::tempdir().unwrap();
    let slot = ServerSlot::default();
    let builds = Arc::new(AtomicUsize::new(0));
    let requested = names(&["alpha"]);
    let mut servers = Vec::new();
    for _ in 0..3 {
        let builds = builds.clone();
        let spec = healthy(dir.path());
        let server = slot
            .ensure(&requested, None, async move {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(spec)
            })
            .await
            .unwrap();
        servers.push(server);
    }
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    assert!(Arc::ptr_eq(&servers[0], &servers[2]));
    let status = slot.status().await;
    assert!(status.running);
    assert!(status.backends[0].ready);
}

#[tokio::test]
async fn a_changed_backend_set_rebuilds_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let slot = ServerSlot::new();
    let first = slot
        .ensure(&names(&["alpha"]), None, async {
            Ok(healthy(dir.path()))
        })
        .await
        .unwrap();
    let second = slot
        .ensure(&names(&["alpha", "beta"]), None, async {
            let mut spec = healthy(dir.path());
            spec.backends = names(&["alpha", "beta"]);
            Ok(spec)
        })
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(second.backends(), names(&["alpha", "beta"]).as_slice());
}

#[tokio::test]
async fn an_idle_heavy_backend_rebuilds_the_worker_but_a_busy_or_other_one_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let slot = ServerSlot::new();
    let requested = names(&["alpha"]);
    let first = slot
        .ensure(&requested, None, async { Ok(healthy(dir.path())) })
        .await
        .unwrap();

    // Not idle long enough.
    let kept = slot
        .ensure(
            &requested,
            Some(IdleRule {
                backend: "alpha",
                timeout: Duration::from_secs(3600),
            }),
            async { panic!("must not rebuild") },
        )
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&first, &kept));

    // Idle, but the rule names a backend this worker does not run.
    let kept = slot
        .ensure(
            &requested,
            Some(IdleRule {
                backend: "beta",
                timeout: Duration::ZERO,
            }),
            async { panic!("must not rebuild") },
        )
        .await
        .unwrap();
    assert!(Arc::ptr_eq(&first, &kept));

    // Idle and named: rebuilt.
    let rebuilt = slot
        .ensure(
            &requested,
            Some(IdleRule {
                backend: "alpha",
                timeout: Duration::ZERO,
            }),
            async { Ok(healthy(dir.path())) },
        )
        .await
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &rebuilt));
}

#[tokio::test]
async fn a_failed_start_is_remembered_and_not_retried() {
    let dir = tempfile::tempdir().unwrap();
    let slot = ServerSlot::new();
    let requested = names(&["alpha"]);
    let error = slot
        .ensure(&requested, None, async {
            Err(Error::Prepare("venv is broken".to_string()))
        })
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "runtime python server unavailable: venv is broken"
    );

    // Within the back-off, the previous failure comes back and nothing runs.
    let error = slot
        .ensure(&requested, None, async {
            panic!("must not retry inside the back-off")
        })
        .await
        .unwrap_err();
    assert!(
        error
            .to_string()
            .starts_with("runtime python server unavailable after previous startup failure"),
        "{error}"
    );

    let status = slot.status().await;
    assert!(status.enabled && !status.running);
    assert!(status.message.unwrap().contains("venv is broken"));
    drop(dir);
}

#[tokio::test]
async fn a_worker_that_will_not_start_is_a_failure_too() {
    let dir = tempfile::tempdir().unwrap();
    let slot = ServerSlot::new();
    let error = slot
        .ensure(&names(&["alpha"]), None, async {
            Ok(launch(dir.path(), "exit 0\n"))
        })
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Unavailable(_)), "{error}");
}

#[tokio::test]
async fn a_cached_worker_that_dies_backs_off() {
    let dir = tempfile::tempdir().unwrap();
    let slot = ServerSlot::new();
    let requested = names(&["alpha"]);
    let server = slot
        .ensure(&requested, None, async { Ok(healthy(dir.path())) })
        .await
        .unwrap();
    // Reset the child, then make its script unstartable: the next `ensure`
    // finds the cached worker, cannot start it, and falls back.
    server.reset().await;
    std::fs::write(dir.path().join("worker.sh"), "exit 0\n").unwrap();
    let error = slot
        .ensure(&requested, None, async { panic!("must not rebuild") })
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Unavailable(_)), "{error}");
    let error = slot
        .ensure(&requested, None, async { panic!("must not rebuild") })
        .await
        .unwrap_err();
    assert!(matches!(error, Error::BackingOff(_)), "{error}");
}
