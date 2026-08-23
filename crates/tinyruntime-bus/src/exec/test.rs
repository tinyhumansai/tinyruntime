//! Unit tests for the execution payloads.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

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
    assert!(
        !timed_out.success(),
        "a job aborted at its deadline did not succeed"
    );
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

#[test]
fn a_response_reports_the_toolchain_that_ran_the_job() {
    // Callers log this; a job that ran on an unexpected version is worth seeing.
    let response = ExecResponse::new("", "", Some(0), "22.11.0");
    assert_eq!(response.runtime_version, "22.11.0");
}

#[test]
fn the_response_wire_form_is_pinned() {
    let value = serde_json::to_value(ExecResponse::new("out", "err", Some(2), "1.0"))
        .expect("response serialises");
    assert_eq!(
        value,
        serde_json::json!({
            "stdout": "out",
            "stderr": "err",
            "exit_code": 2,
            "timed_out": false,
            "elapsed_ms": 0,
            "queue_wait_ms": 0,
            "runtime_version": "1.0",
        })
    );
}

#[test]
fn a_response_round_trips_across_the_wire() {
    let response = ExecResponse::new("out", "", None, "1.0")
        .with_timed_out(true)
        .with_timings(5, 6);
    let decoded: ExecResponse =
        serde_json::from_value(serde_json::to_value(&response).expect("serialises"))
            .expect("round-trips");
    assert_eq!(decoded, response);
}
