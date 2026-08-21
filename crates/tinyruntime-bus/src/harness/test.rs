//! Unit tests for the worker harness payload.

use super::{WORKER_PROTOCOL_VERSION, WorkerHarness};

#[test]
fn a_new_harness_announces_the_compiled_in_protocol() {
    let harness = WorkerHarness::new("pool_worker.py", "# ...", "python");
    assert_eq!(harness.protocol_version, WORKER_PROTOCOL_VERSION);
    assert!(harness.args_before_script.is_empty());
}

#[test]
fn command_args_puts_flags_before_the_script_and_extras_after() {
    let mut harness = WorkerHarness::new("pool_worker.js", "// ...", "node")
        .with_flag("--experimental-vm-modules")
        .with_flag("--experimental-import-meta-resolve");
    harness.args_after_script.push("--serve".to_string());

    assert_eq!(
        harness.command_args("/cache/pool_worker.js"),
        vec![
            "--experimental-vm-modules".to_string(),
            "--experimental-import-meta-resolve".to_string(),
            "/cache/pool_worker.js".to_string(),
            "--serve".to_string(),
        ]
    );
}

#[test]
fn extra_environment_rides_along_with_the_harness() {
    let harness =
        WorkerHarness::new("pool_worker.py", "# ...", "python").with_env("PYTHONUNBUFFERED", "1");
    assert_eq!(
        harness.env,
        vec![("PYTHONUNBUFFERED".to_string(), "1".to_string())]
    );
}

#[test]
fn round_trips_across_the_wire() {
    let harness = WorkerHarness::new("pool_worker.js", "// harness", "node").with_flag("--flag");
    let value = serde_json::to_value(&harness).expect("harness serialises");
    let decoded: WorkerHarness = serde_json::from_value(value).expect("harness round-trips");
    assert_eq!(decoded, harness);
}
