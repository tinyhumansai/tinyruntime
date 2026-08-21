//! Unit tests for the worker environment.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use tinyruntime_bus::WorkerHarness;

use super::{build, materialise};

#[test]
fn the_toolchain_directory_comes_first_on_path() {
    let env = build(Path::new("/managed/bin"), &[]);
    let path = env
        .iter()
        .find(|(name, _)| name == "PATH")
        .map(|(_, value)| value.clone())
        .expect("a worker always gets a PATH");
    assert!(path.starts_with("/managed/bin"), "got `{path}`");
}

#[test]
fn a_secret_in_this_process_does_not_reach_a_worker() {
    // The whole reason the environment is an allow-list: this module is loaded
    // into a host that holds credentials, and a worker runs code that must not
    // be able to read them.
    // SAFETY-equivalent note: single-threaded test, no other thread reads the env.
    unsafe {
        std::env::set_var("TINYRUNTIME_TEST_SECRET", "super-secret-token");
    }
    let env = build(Path::new("/managed/bin"), &[]);
    unsafe {
        std::env::remove_var("TINYRUNTIME_TEST_SECRET");
    }

    assert!(
        !env.iter().any(|(name, _)| name == "TINYRUNTIME_TEST_SECRET"),
        "an unlisted variable leaked into the worker environment"
    );
}

#[test]
fn a_providers_extra_variables_are_added() {
    let env = build(
        Path::new("/managed/bin"),
        &[("PYTHONUNBUFFERED".to_string(), "1".to_string())],
    );
    assert!(
        env.iter()
            .any(|(name, value)| name == "PYTHONUNBUFFERED" && value == "1")
    );
}

#[tokio::test]
async fn the_harness_is_written_where_the_worker_can_be_launched_from() {
    let scratch = tempfile::tempdir().unwrap();
    let harness = WorkerHarness::new("pool_worker.js", "// harness body", "node");

    let path = materialise(scratch.path(), &harness)
        .await
        .expect("the harness is written");

    assert_eq!(path.file_name().unwrap(), "pool_worker.js");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "// harness body");
}

#[tokio::test]
async fn rewriting_replaces_a_stale_harness() {
    // A provider that ships a new harness after an upgrade must not be shadowed
    // by the previous one still on disk.
    let scratch = tempfile::tempdir().unwrap();
    materialise(
        scratch.path(),
        &WorkerHarness::new("pool_worker.js", "old", "node"),
    )
    .await
    .unwrap();
    let path = materialise(
        scratch.path(),
        &WorkerHarness::new("pool_worker.js", "new", "node"),
    )
    .await
    .unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
}
