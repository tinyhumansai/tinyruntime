//! Unit tests for the engine.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use tinyruntime_bus::{
    ExecRequest, Language, ResolveRequest, RuntimeLayout, RuntimeSettings, WorkerHarness,
};

use super::Engine;
use crate::error::Error;
use crate::provider::stub::StubProvider;
use crate::provider::{Provider, Registry};

fn engine_over(provider: Arc<dyn Provider>, harness_root: &std::path::Path) -> Engine {
    let mut registry = Registry::new();
    registry.register(Language::nodejs(), "ai.example.Provider", provider);
    Engine::new(registry, reqwest::Client::new(), harness_root.to_path_buf())
}

fn settings(cache_dir: &std::path::Path) -> RuntimeSettings {
    let mut settings = RuntimeSettings::new("1.0.0");
    settings.cache_dir = cache_dir.to_string_lossy().into_owned();
    settings
}

#[tokio::test]
async fn an_engine_routing_nothing_has_an_empty_registry() {
    let scratch = tempfile::tempdir().unwrap();
    let engine = Engine::new(
        Registry::new(),
        reqwest::Client::new(),
        scratch.path().to_path_buf(),
    );
    assert!(engine.registry().is_empty());
    assert!(engine.pool_stats().await.is_empty());
}

#[tokio::test]
async fn resolution_is_reachable_without_running_anything() {
    let scratch = tempfile::tempdir().unwrap();
    let provider = Arc::new(
        StubProvider::new(Language::nodejs()).with_system(
            RuntimeLayout::new("1.2.3", "/usr/local/bin")
                .with_executable("tool", "/usr/local/bin/tool"),
        ),
    );
    let engine = engine_over(provider, scratch.path());

    let resolved = engine
        .resolve(&ResolveRequest::probe(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect("resolution succeeds")
        .expect("the host toolchain was found");
    assert_eq!(resolved.version, "1.2.3");
}

#[tokio::test]
async fn executing_an_unprovisioned_language_fails_before_a_pool_is_built() {
    // A failed resolution must not leave a pool behind: pools hold interpreter
    // children, and one built for a toolchain that does not exist would spawn
    // nothing and report counters for a language that never ran.
    let scratch = tempfile::tempdir().unwrap();
    let engine = engine_over(
        Arc::new(StubProvider::new(Language::nodejs())),
        scratch.path(),
    );

    let request = ExecRequest::new(Language::nodejs(), settings(scratch.path()), "1 + 1");
    let error = engine
        .execute(&request)
        .await
        .expect_err("nothing to run on");

    assert!(matches!(error, Error::Download { .. }), "got {error:?}");
    assert!(engine.pool_stats().await.is_empty());
}

#[tokio::test]
async fn a_toolchain_missing_the_harness_executable_is_reported_as_an_empty_install() {
    // The provider resolved a toolchain and then asked to launch a binary that
    // toolchain does not ship. Spawning would fail much later with a confusing
    // message, so it is caught while the cause is still obvious.
    let scratch = tempfile::tempdir().unwrap();
    let provider = Arc::new(
        StubProvider::new(Language::nodejs())
            .with_system(
                RuntimeLayout::new("1.2.3", "/usr/local/bin")
                    .with_executable("something-else", "/usr/local/bin/something-else"),
            )
            .with_harness(WorkerHarness::new("pool_worker.js", "// harness", "node")),
    );
    let engine = engine_over(provider, scratch.path());

    let request = ExecRequest::new(Language::nodejs(), settings(scratch.path()), "1 + 1");
    let error = engine
        .execute(&request)
        .await
        .expect_err("nothing to launch");
    assert!(matches!(error, Error::EmptyInstall(_)), "got {error:?}");
}

#[tokio::test]
async fn a_provider_with_no_harness_cannot_execute() {
    let scratch = tempfile::tempdir().unwrap();
    let provider = Arc::new(
        StubProvider::new(Language::nodejs()).with_system(
            RuntimeLayout::new("1.2.3", "/usr/local/bin")
                .with_executable("node", "/usr/local/bin/node"),
        ),
    );
    let engine = engine_over(provider, scratch.path());

    let request = ExecRequest::new(Language::nodejs(), settings(scratch.path()), "1 + 1");
    let error = engine
        .execute(&request)
        .await
        .expect_err("no harness to launch");
    assert!(
        matches!(error, Error::ProviderUnavailable { .. }),
        "got {error:?}"
    );
}
