//! Persistent worker fixtures exercised by a trusted Rust host, without bus exposure.

#[cfg(unix)]
#[tokio::test]
async fn trusted_host_dispatches_real_jsonl_and_cleanup() -> Result<(), String> {
    use tinyruntime_bus::worker::{
        WorkerCommand, WorkerHandle, WorkerOutcome, WorkerOutcomeKind, WorkerPlan, WorkerPrepare,
        WorkerRequest, WorkerStatus,
    };
    // Darwin's default temp path contains the /var -> /private/var alias.
    // Use its canonical root so the worker's no-symlink policy is exercised.
    let temp_root = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let scratch = tempfile::tempdir_in(temp_root).map_err(|error| error.to_string())?;
    let scopes = vec![tinyruntime_bus::worker::WorkerCacheScope {
        id: "cache".into(),
        root: scratch
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .join("recipe-cache")
            .to_string_lossy()
            .into_owned(),
    }];
    let workers = tinyruntime::worker::WorkerManager::new(scratch.path().join("workers"))
        .with_cache_scopes(scopes)
        .map_err(|error| error.to_string())?;
    let handle: WorkerHandle = workers
        .reserve()
        .await
        .map_err(|error| format!("{error:?}"))?;
    let source = r#"printf '%s\n' '{"ready":true,"protocol":1,"backends":["alpha"]}'
while IFS= read -r line; do
 id=${line#*\"id\":\"}; id=${id%%\"*}
 printf '{"id":"%s","ok":true,"result":{"answer":42}}\n' "$id"
done
"#;
    let plan = WorkerPlan {
        source: source.into(),
        command: WorkerCommand {
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
    };
    let prepared: WorkerOutcome = workers
        .prepare(WorkerPrepare {
            handle: handle.clone(),
            plan: plan.clone(),
        })
        .await;
    assert_eq!(prepared.kind, WorkerOutcomeKind::Complete, "{prepared:?}");
    let started: WorkerOutcome = workers.start(&handle).await;
    assert_eq!(started.kind, WorkerOutcomeKind::Complete);
    let status: WorkerStatus = workers
        .status(&handle)
        .await
        .map_err(|error| format!("{error:?}"))?;
    assert!(status.server.running);
    assert!(status.server.backends[0].ready);
    let response: WorkerOutcome = workers
        .request(WorkerRequest {
            handle: handle.clone(),
            operation: 1,
            method: "alpha.run".into(),
            params: serde_json::json!({"input":"fixture"}),
        })
        .await;
    assert_eq!(
        response.response.ok_or("missing worker response")?.result,
        Some(serde_json::json!({"answer":42}))
    );
    let stopped: WorkerOutcome = workers.stop(&handle).await;
    assert_eq!(stopped.kind, WorkerOutcomeKind::Complete);
    let shutdown: WorkerOutcome = workers.shutdown().await;
    assert_eq!(shutdown.kind, WorkerOutcomeKind::Complete);
    assert!(workers.reserve().await.is_err());
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn trusted_host_publishes_exact_marker_bytes() -> Result<(), String> {
    use tinyruntime_bus::worker::{
        WorkerCommand, WorkerHandle, WorkerOutcome, WorkerOutcomeKind, WorkerPlan,
        WorkerPrepareCached,
    };
    // Darwin's default temp path contains the /var -> /private/var alias.
    // Use its canonical root so the worker's no-symlink policy is exercised.
    let temp_root = std::env::temp_dir()
        .canonicalize()
        .map_err(|error| error.to_string())?;
    let scratch = tempfile::tempdir_in(temp_root).map_err(|error| error.to_string())?;
    let scopes = vec![tinyruntime_bus::worker::WorkerCacheScope {
        id: "cache".into(),
        root: scratch
            .path()
            .canonicalize()
            .map_err(|error| error.to_string())?
            .join("recipe-cache")
            .to_string_lossy()
            .into_owned(),
    }];
    let workers = tinyruntime::worker::WorkerManager::new(scratch.path().join("workers"))
        .with_cache_scopes(scopes)
        .map_err(|error| error.to_string())?;
    let handle: WorkerHandle = workers
        .reserve()
        .await
        .map_err(|error| format!("{error:?}"))?;
    let prepared: WorkerOutcome = workers
        .prepare_cached(WorkerPrepareCached {
            handle: handle.clone(),
            plan: WorkerPlan {
                source: "exit 0".into(),
                command: WorkerCommand {
                    executable: "/bin/sh".into(),
                    args: Vec::new(),
                    env: Vec::new(),
                    timeout_ms: None,
                },
                preparation: Vec::new(),
                backends: Vec::new(),
                startup_timeout_ms: 1000,
                request_timeout_ms: 1000,
                idle_backend: None,
                idle_timeout_ms: 0,
            },
            recipe: tinyruntime_bus::worker::CacheRecipe {
                scope: "cache".into(),
                artifacts: Vec::new(),
                steps: Vec::new(),
                required: Vec::new(),
                marker: tinyruntime_bus::worker::CacheArtifact {
                    path: "ready".into(),
                    bytes: b"exact".to_vec(),
                },
                adoption: tinyruntime_bus::worker::CacheAdoptionPolicy::Strict,
                timeout_ms: 1000,
            },
        })
        .await;
    assert_eq!(prepared.kind, WorkerOutcomeKind::Complete, "{prepared:?}");
    assert_eq!(
        std::fs::read(scratch.path().join("recipe-cache/ready"))
            .map_err(|error| error.to_string())?,
        b"exact"
    );
    let stopped: WorkerOutcome = workers.stop(&handle).await;
    assert_eq!(stopped.kind, WorkerOutcomeKind::Complete);
    Ok(())
}
