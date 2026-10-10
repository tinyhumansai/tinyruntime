//! Persistent cache preparation preserves ownership, reuse and final locations.
#![allow(
    clippy::unwrap_used,
    reason = "fixture assertions match existing test suites"
)]
#[cfg(unix)]
use super::tests::{HEALTHY, plan};
use super::*;
use tinyruntime_bus::worker::{
    CacheAdoptionPolicy, CacheArtifact, CacheEntry, CacheEntryKind, WorkerCommand,
};

fn recipe(steps: Vec<WorkerCommand>) -> CacheRecipe {
    CacheRecipe {
        scope: "approved".into(),
        artifacts: vec![CacheArtifact {
            path: "input".into(),
            bytes: b"data".to_vec(),
        }],
        steps,
        required: vec![CacheEntry {
            path: "payload".into(),
            kind: CacheEntryKind::File,
        }],
        marker: CacheArtifact {
            path: "ready".into(),
            bytes: b"v1".to_vec(),
        },
        adoption: CacheAdoptionPolicy::Strict,
        timeout_ms: 30_000,
    }
}

fn manager(root: &std::path::Path) -> WorkerManager {
    let root = root.canonicalize().unwrap();
    WorkerManager::new(root.join("workers"))
        .with_cache_scopes(vec![WorkerCacheScope {
            id: "approved".into(),
            root: root.join("cache").to_string_lossy().into_owned(),
        }])
        .unwrap()
}

#[cfg(unix)]
fn step(source: &str) -> WorkerCommand {
    WorkerCommand {
        executable: "/bin/sh".into(),
        args: vec!["-c".into(), source.into()],
        env: Vec::new(),
    }
}

#[cfg(unix)]
async fn prepare(manager: &WorkerManager, recipe: CacheRecipe) -> (WorkerHandle, WorkerOutcome) {
    let handle = manager.reserve().await.unwrap();
    let result = manager
        .prepare_cached(WorkerPrepareCached {
            handle: handle.clone(),
            plan: plan(HEALTHY),
            recipe,
        })
        .await;
    (handle, result)
}

#[cfg(unix)]
async fn wait_file(path: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(5), async {
        while !path.exists() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn successful_cache_is_reused_after_manager_restart_without_native_commands() {
    let root = tempfile::tempdir().unwrap();
    let recipe = recipe(vec![step("printf 'run\\n' >> count; cat input > payload")]);
    let first = manager(root.path());
    let (handle, result) = prepare(&first, recipe.clone()).await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    assert_eq!(first.start(&handle).await.kind, WorkerOutcomeKind::Complete);
    assert_eq!(first.shutdown().await.kind, WorkerOutcomeKind::Complete);
    let second = manager(root.path());
    let (_, result) = prepare(&second, recipe).await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    assert_eq!(
        std::fs::read(root.path().join("cache/count")).unwrap(),
        b"run\n"
    );
    assert_eq!(
        std::fs::read(root.path().join("cache/ready")).unwrap(),
        b"v1"
    );
    assert_eq!(second.shutdown().await.kind, WorkerOutcomeKind::Complete);
}

#[cfg(unix)]
#[tokio::test]
async fn partial_stale_and_changed_artifact_caches_are_reprovisioned_under_the_lock() {
    let root = tempfile::tempdir().unwrap();
    let cache = root.path().join("cache");
    std::fs::create_dir(&cache).unwrap();
    let recipe = recipe(vec![step("printf 'run\\n' >> count; cat input > payload")]);
    for phase in 0..4 {
        match phase {
            0 => {
                std::fs::write(cache.join("ready"), b"v1").unwrap();
            }
            1 => {
                std::fs::write(cache.join("ready"), b"stale").unwrap();
            }
            2 => {
                std::fs::write(cache.join("input"), b"changed").unwrap();
            }
            _ => {
                std::fs::remove_file(cache.join("ready")).unwrap();
            }
        }
        let manager = manager(root.path());
        let (_, result) = prepare(&manager, recipe.clone()).await;
        assert_eq!(result.kind, WorkerOutcomeKind::Complete);
        manager.shutdown().await;
    }
    assert_eq!(
        std::fs::read(cache.join("count")).unwrap(),
        b"run\nrun\nrun\nrun\n"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn parallel_managers_provision_one_cache_once_then_observe_the_same_marker() {
    let root = tempfile::tempdir().unwrap();
    let first = manager(root.path());
    let second = manager(root.path());
    let recipe = recipe(vec![step(
        "printf started > started; while [ ! -f release ]; do :; done; printf run >> count; cat input > payload",
    )]);
    let one = {
        let first = first.clone();
        let recipe = recipe.clone();
        tokio::spawn(async move { prepare(&first, recipe).await })
    };
    wait_file(&root.path().join("cache/started")).await;
    let two = {
        let second = second.clone();
        let recipe = recipe.clone();
        tokio::spawn(async move { prepare(&second, recipe).await })
    };
    std::fs::write(root.path().join("cache/release"), b"go").unwrap();
    assert_eq!(one.await.unwrap().1.kind, WorkerOutcomeKind::Complete);
    assert_eq!(two.await.unwrap().1.kind, WorkerOutcomeKind::Complete);
    assert_eq!(
        std::fs::read(root.path().join("cache/count")).unwrap(),
        b"run"
    );
    first.shutdown().await;
    second.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn failed_recipe_preserves_partial_cache_and_allows_retries_beyond_capacity() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path());
    let failed = recipe(vec![step(
        "printf user > preserved; printf partial > payload; exit 1",
    )]);
    for _ in 0..32 {
        let (handle, result) = prepare(&manager, failed.clone()).await;
        assert_eq!(result.kind, WorkerOutcomeKind::Failed);
        assert!(!root.path().join("cache/ready").exists());
        assert_eq!(
            manager.stop(&handle).await.kind,
            WorkerOutcomeKind::Complete
        );
    }
    let (_, result) = prepare(&manager, recipe(vec![step("cat input > payload")])).await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    manager.shutdown().await;
    assert_eq!(
        std::fs::read(root.path().join("cache/preserved")).unwrap(),
        b"user"
    );
    assert!(
        !std::fs::read_dir(root.path().join("cache"))
            .unwrap()
            .any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tinyruntime-stage-"))
    );
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn abandoned_cache_reply_and_shutdown_reap_preparation_before_unlock_and_no_marker() {
    let root = tempfile::tempdir().unwrap();
    let first = manager(root.path());
    let pending = {
        let first = first.clone();
        tokio::spawn(async move {
            prepare(
                &first,
                recipe(vec![step(
                    "printf '%s' $$ > pid; printf partial > payload; exec sleep 600",
                )]),
            )
            .await
        })
    };
    wait_file(&root.path().join("cache/pid")).await;
    let pid = std::fs::read_to_string(root.path().join("cache/pid")).unwrap();
    pending.abort();
    let _ = pending.await;
    assert_eq!(first.shutdown().await.kind, WorkerOutcomeKind::Complete);
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    assert!(!root.path().join("cache/ready").exists());
    assert_eq!(
        std::fs::read(root.path().join("cache/payload")).unwrap(),
        b"partial"
    );
    let second = manager(root.path());
    assert_eq!(
        prepare(&second, recipe(vec![step("cat input > payload")]))
            .await
            .1
            .kind,
        WorkerOutcomeKind::Complete
    );
    second.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn injected_user_staging_file_remains_owned_by_user_and_cleanup_is_publicly_retryable() {
    let root = tempfile::tempdir().unwrap();
    let manager = manager(root.path());
    let (handle, result) = prepare(&manager, recipe(vec![step("for stage in .tinyruntime-stage-*; do printf user > \"$stage/user\"; done; cat input > payload")])).await;
    assert_eq!(result.kind, WorkerOutcomeKind::Failed);
    assert_eq!(manager.stop(&handle).await.kind, WorkerOutcomeKind::Failed);
    let stage = std::fs::read_dir(root.path().join("cache"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".tinyruntime-stage-")
        })
        .unwrap();
    assert_eq!(std::fs::read(stage.join("user")).unwrap(), b"user");
    std::fs::remove_file(stage.join("user")).unwrap();
    assert_eq!(
        manager.stop(&handle).await.kind,
        WorkerOutcomeKind::Complete
    );
}

#[cfg(unix)]
#[tokio::test]
async fn changed_preparation_recipe_invalidates_readiness_without_changing_marker_bytes() {
    let root = tempfile::tempdir().unwrap();
    let first = manager(root.path());
    let (_, result) = prepare(
        &first,
        recipe(vec![step("printf first >> count; cat input > payload")]),
    )
    .await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    first.shutdown().await;
    let second = manager(root.path());
    let (_, result) = prepare(
        &second,
        recipe(vec![step("printf second >> count; cat input > payload")]),
    )
    .await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    second.shutdown().await;
    assert_eq!(
        std::fs::read(root.path().join("cache/count")).unwrap(),
        b"firstsecond"
    );
    assert_eq!(
        std::fs::read(root.path().join("cache/ready")).unwrap(),
        b"v1"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn failed_changed_recipe_preserves_previous_identity_instead_of_becoming_legacy() {
    let root = tempfile::tempdir().unwrap();
    let first = manager(root.path());
    assert_eq!(
        prepare(&first, recipe(vec![step("cat input > payload")]))
            .await
            .1
            .kind,
        WorkerOutcomeKind::Complete
    );
    first.shutdown().await;
    let identity = std::fs::read_dir(root.path().join("cache"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".tinyruntime-recipe-")
        })
        .unwrap();
    let previous = std::fs::read(&identity).unwrap();
    let second = manager(root.path());
    let (handle, result) = prepare(&second, recipe(vec![step("exit 1")])).await;
    assert_eq!(result.kind, WorkerOutcomeKind::Failed);
    second.stop(&handle).await;
    assert_eq!(std::fs::read(&identity).unwrap(), previous);
    assert!(!root.path().join("cache/ready").exists());
}

#[cfg(unix)]
fn remove_identity(cache: &std::path::Path) {
    let identity = std::fs::read_dir(cache)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(".tinyruntime-recipe-")
        })
        .unwrap();
    std::fs::remove_file(identity).unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn legacy_cache_adoption_is_default_denied_and_requires_exact_readiness() {
    let root = tempfile::tempdir().unwrap();
    let initial = manager(root.path());
    let (_, result) = prepare(
        &initial,
        recipe(vec![step("printf first >> count; cat input > payload")]),
    )
    .await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    initial.shutdown().await;
    let cache = root.path().join("cache");
    remove_identity(&cache);

    let strict = manager(root.path());
    let (_, result) = prepare(
        &strict,
        recipe(vec![step("printf strict >> count; cat input > payload")]),
    )
    .await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    strict.shutdown().await;
    assert_eq!(std::fs::read(cache.join("count")).unwrap(), b"firststrict");
}

#[cfg(unix)]
#[tokio::test]
async fn explicitly_adopts_only_verified_legacy_cache_without_running_commands() {
    let root = tempfile::tempdir().unwrap();
    let initial = manager(root.path());
    let (_, result) = prepare(
        &initial,
        recipe(vec![step("printf first >> count; cat input > payload")]),
    )
    .await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    initial.shutdown().await;
    let cache = root.path().join("cache");
    remove_identity(&cache);

    let adoption_manager = manager(root.path());
    let mut legacy_recipe = recipe(vec![step("printf adopted >> count; cat input > payload")]);
    legacy_recipe.adoption = CacheAdoptionPolicy::AdoptVerifiedLegacy;
    let (_, result) = prepare(&adoption_manager, legacy_recipe).await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    adoption_manager.shutdown().await;
    assert_eq!(std::fs::read(cache.join("count")).unwrap(), b"first");
    assert!(std::fs::read_dir(&cache).unwrap().any(|entry| {
        entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".tinyruntime-recipe-")
    }));
}

#[cfg(unix)]
#[tokio::test]
async fn explicit_legacy_policy_never_adopts_a_mismatched_existing_identity() {
    let root = tempfile::tempdir().unwrap();
    let initial = manager(root.path());
    let (_, result) = prepare(
        &initial,
        recipe(vec![step("printf first >> count; cat input > payload")]),
    )
    .await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    initial.shutdown().await;

    let rebuilt = manager(root.path());
    let mut changed = recipe(vec![step("printf changed >> count; cat input > payload")]);
    changed.adoption = CacheAdoptionPolicy::AdoptVerifiedLegacy;
    let (_, result) = prepare(&rebuilt, changed).await;
    assert_eq!(result.kind, WorkerOutcomeKind::Complete);
    rebuilt.shutdown().await;
    assert_eq!(
        std::fs::read(root.path().join("cache/count")).unwrap(),
        b"firstchanged"
    );
}
