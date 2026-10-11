//! Cache ownership and cross-process lock fixtures.
#![allow(
    clippy::unwrap_used,
    reason = "local fixture assertions match existing test conventions"
)]
use super::*;

#[test]
fn duplicate_and_overlapping_scope_admission_is_bounded() {
    let scope = WorkerCacheScope {
        id: "cache".into(),
        root: "/tmp/cache".into(),
    };
    assert!(scopes(std::slice::from_ref(&scope)).is_ok());
    assert!(scopes(&[scope.clone(), scope.clone()]).is_err());
    let mut nested = scope.clone();
    nested.id = "nested".into();
    nested.root.push_str("/child");
    assert!(scopes(&[scope.clone(), nested]).is_err());
    assert!(scopes(&vec![scope; 33]).is_err());
}

fn recipe() -> CacheRecipe {
    CacheRecipe {
        scope: "cache".into(),
        artifacts: Vec::new(),
        steps: Vec::new(),
        required: Vec::new(),
        marker: tinyruntime_bus::worker::CacheArtifact {
            path: "ready".into(),
            bytes: b"v1".to_vec(),
        },
        adoption: tinyruntime_bus::worker::CacheAdoptionPolicy::Strict,
        timeout_ms: 1000,
    }
}

#[tokio::test]
async fn cache_lock_wait_observes_deadline_and_cancellation_without_stealing_lock() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path().canonicalize().unwrap();
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".tinyruntime-cache.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    let (stop, mut receiver) = watch::channel(false);
    tokio::time::pause();
    let result = Session::acquire(
        &root,
        &recipe(),
        &mut receiver,
        Instant::now() + Duration::from_secs(1),
    )
    .await;
    assert_eq!(result.unwrap_err(), "cache_timeout");
    stop.send_replace(true);
    assert_eq!(
        Session::acquire(
            &root,
            &recipe(),
            &mut receiver,
            Instant::now() + Duration::from_secs(1)
        )
        .await
        .unwrap_err(),
        "canceled"
    );
    drop(lock);
    stop.send_replace(false);
    let mut session = Session::acquire(
        &root,
        &recipe(),
        &mut receiver,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    session.install(&recipe()).unwrap();
    session.publish(&recipe()).unwrap();
    session.cleanup().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn scope_and_lock_replacements_refuse_readiness_publication_without_outside_writes() {
    let scratch = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let root = scratch.path().canonicalize().unwrap().join("cache");
    let (_stop, mut receiver) = watch::channel(false);
    let mut session = Session::acquire(
        &root,
        &recipe(),
        &mut receiver,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    session.install(&recipe()).unwrap();
    std::fs::rename(&root, scratch.path().join("old")).unwrap();
    std::os::unix::fs::symlink(outside.path(), &root).unwrap();
    assert!(session.publish(&recipe()).is_err());
    assert!(!outside.path().join("ready").exists());
    session.cleanup().unwrap();
    std::fs::remove_file(&root).unwrap();
    std::fs::rename(scratch.path().join("old"), &root).unwrap();
    let mut session = Session::acquire(
        &root,
        &recipe(),
        &mut receiver,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    session.install(&recipe()).unwrap();
    std::fs::rename(root.join(".tinyruntime-cache.lock"), root.join("old-lock")).unwrap();
    std::fs::write(root.join(".tinyruntime-cache.lock"), b"replacement").unwrap();
    assert_eq!(session.check_binding().unwrap_err(), "cache_lock_changed");
    assert!(session.publish(&recipe()).is_err());
    session.cleanup().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unsafe_lock_file_and_missing_required_entries_are_not_cached_successes() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().canonicalize().unwrap();
    let (_stop, mut receiver) = watch::channel(false);
    std::os::unix::fs::symlink(root.join("outside"), root.join(".tinyruntime-cache.lock")).unwrap();
    assert_eq!(
        Session::acquire(
            &root,
            &recipe(),
            &mut receiver,
            Instant::now() + Duration::from_secs(1)
        )
        .await
        .unwrap_err(),
        "cache_lock_failed"
    );
    std::fs::remove_file(root.join(".tinyruntime-cache.lock")).unwrap();
    let mut value = recipe();
    value.required.push(tinyruntime_bus::worker::CacheEntry {
        path: "missing".into(),
        kind: tinyruntime_bus::worker::CacheEntryKind::File,
    });
    let mut session = Session::acquire(
        &root,
        &value,
        &mut receiver,
        Instant::now() + Duration::from_secs(1),
    )
    .await
    .unwrap();
    session.install(&value).unwrap();
    assert_eq!(
        session.publish(&value).unwrap_err(),
        "cache_required_missing"
    );
    session.cleanup().unwrap();
    assert!(!root.join("ready").exists());
}
