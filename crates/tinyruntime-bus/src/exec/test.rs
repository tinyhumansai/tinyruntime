//! Unit tests for the execution payloads.

use super::{ExecRequest, ExecResponse};
use crate::{Language, RuntimeSettings};

#[test]
fn a_request_defaults_to_the_standard_pool_and_no_deadline() {
    let request = ExecRequest::new(
        Language::nodejs(),
        RuntimeSettings::new("v22.11.0"),
        "console.log(1)",
    );
    assert_eq!(request.pool, crate::PoolSettings::default());
    assert!(request.timeout_ms.is_none());
    assert!(request.cwd.is_none());
}

#[test]
fn builders_set_the_working_directory_and_the_deadline() {
    let request = ExecRequest::new(Language::python(), RuntimeSettings::new("3.12"), "print(1)")
        .with_cwd("/work/sandbox")
        .with_timeout_ms(5_000);
    assert_eq!(request.cwd.as_deref(), Some("/work/sandbox"));
    assert_eq!(request.timeout_ms, Some(5_000));
}

#[test]
fn success_requires_a_clean_exit_and_no_timeout() {
    assert!(ExecResponse::new("", "", Some(0), "1.0").success());
    assert!(ExecResponse::new("", "", None, "1.0").success());
    assert!(!ExecResponse::new("", "boom", Some(1), "1.0").success());

    let timed_out = ExecResponse {
        timed_out: true,
        ..ExecResponse::new("", "", Some(0), "1.0")
    };
    assert!(!timed_out.success(), "a job aborted at its deadline did not succeed");
}

#[test]
fn queue_wait_is_reported_apart_from_run_time() {
    let response = ExecResponse {
        elapsed_ms: 12,
        queue_wait_ms: 900,
        ..ExecResponse::new("", "", Some(0), "1.0")
    };
    let value = serde_json::to_value(&response).expect("response serialises");
    assert_eq!(value["elapsed_ms"], 12);
    assert_eq!(value["queue_wait_ms"], 900);
}

#[test]
fn the_request_wire_form_omits_nothing_a_worker_needs() {
    let request = ExecRequest::new(Language::nodejs(), RuntimeSettings::new("v22"), "1");
    let value = serde_json::to_value(&request).expect("request serialises");
    for field in ["language", "settings", "pool", "code", "cwd", "timeout_ms"] {
        assert!(value.get(field).is_some(), "missing field {field}");
    }
}
