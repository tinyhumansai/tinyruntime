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
    crate::testing::evaluate_log_fields();
    let mut registry = Registry::new();
    registry.register(&Language::nodejs(), "ai.example.Provider", provider);
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

// ---------------------------------------------------------------------------
// The whole path, end to end
//
// Resolve, build a launch from the provider's harness, start a pool, run a job.
// The "interpreter" is this test binary re-executed as a worker, so the engine's
// own wiring is exercised without depending on Node or Python being installed.
// ---------------------------------------------------------------------------

use crate::pool::fake_worker::{self, Directive};

/// The logical executable the fake harness runs under.
const TOOL: &str = "tool";

/// A provider reporting this test binary as the toolchain, with a harness whose
/// flags make it serve the worker protocol.
fn worker_provider() -> Arc<StubProvider> {
    let binary = std::env::current_exe().expect("a test binary has a path");
    let bin_dir = binary
        .parent()
        .expect("the binary is in a directory")
        .to_string_lossy()
        .into_owned();

    let launch = fake_worker::launch(Language::nodejs());
    let mut harness = WorkerHarness::new("worker-harness", "unused by this worker", TOOL)
        .with_env(fake_worker::WORKER_MARKER, "1");
    // The script path `command_args` appends is an extra libtest filter that
    // matches no test, so `--exact` still selects only the worker entry point.
    harness.args_before_script = launch.args.clone();

    Arc::new(
        StubProvider::new(Language::nodejs())
            .with_system(
                RuntimeLayout::new("1.0.0-test", bin_dir)
                    .with_executable(TOOL, binary.to_string_lossy().into_owned()),
            )
            .with_harness(harness),
    )
}

#[tokio::test]
async fn a_job_runs_through_resolution_launch_and_the_pool() {
    let scratch = tempfile::tempdir().unwrap();
    let engine = engine_over(worker_provider(), scratch.path());

    let request = ExecRequest::new(
        Language::nodejs(),
        settings(scratch.path()),
        Directive::Echo("through-the-engine").code(),
    );
    let response = engine.execute(&request).await.expect("the job runs");

    assert_eq!(response.stdout, "through-the-engine");
    assert!(response.success());
    assert_eq!(
        response.runtime_version, "1.0.0-test",
        "the reply carries the resolved toolchain"
    );
}

#[tokio::test]
async fn the_harness_is_written_where_the_worker_is_launched_from() {
    let scratch = tempfile::tempdir().unwrap();
    let engine = engine_over(worker_provider(), scratch.path());

    engine
        .execute(&ExecRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
            Directive::Echo("x").code(),
        ))
        .await
        .expect("the job runs");

    let written = scratch.path().join("nodejs").join("worker-harness");
    assert!(written.is_file(), "the harness was not materialised");
    assert_eq!(
        std::fs::read_to_string(&written).unwrap(),
        "unused by this worker"
    );
}

#[tokio::test]
async fn a_second_job_reuses_the_pool_the_first_one_built() {
    let scratch = tempfile::tempdir().unwrap();
    let engine = engine_over(worker_provider(), scratch.path());
    let request = ExecRequest::new(
        Language::nodejs(),
        settings(scratch.path()),
        Directive::Echo("x").code(),
    );

    engine.execute(&request).await.expect("the first job runs");
    engine.execute(&request).await.expect("the second job runs");

    let stats = engine.pool_stats().await;
    assert_eq!(
        stats.len(),
        1,
        "a second pool was built for the same launch"
    );
    assert_eq!(stats[0].jobs_total, 2);
    assert_eq!(
        stats[0].worker_spawns, 1,
        "two jobs spawned {} interpreters",
        stats[0].worker_spawns
    );
}

#[tokio::test]
async fn a_failing_job_comes_back_as_output_rather_than_an_error() {
    let scratch = tempfile::tempdir().unwrap();
    let engine = engine_over(worker_provider(), scratch.path());

    let response = engine
        .execute(&ExecRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
            Directive::Fail("threw").code(),
        ))
        .await
        .expect("a throwing job still returns");

    assert!(!response.success());
    assert_eq!(response.stderr, "threw");
}

#[tokio::test]
async fn pool_stats_are_empty_until_something_runs() {
    let scratch = tempfile::tempdir().unwrap();
    let engine = engine_over(worker_provider(), scratch.path());
    assert!(engine.pool_stats().await.is_empty());
}
