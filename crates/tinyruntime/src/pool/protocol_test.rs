//! Unit tests for the worker framing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Handshake, JobRequest, JobResponse};

#[test]
fn a_handshake_parses_from_what_a_harness_prints() {
    let handshake: Handshake =
        serde_json::from_str(r#"{"ready":true,"protocol":1,"language":"nodejs","token":"abc"}"#)
            .expect("a handshake parses");
    assert!(handshake.ready);
    assert_eq!(handshake.protocol, Some(1));
    assert_eq!(handshake.token.as_deref(), Some("abc"));
}

#[test]
fn a_failed_startup_arrives_as_a_handshake_rather_than_a_closed_stream() {
    // A worker that cannot start should say so, so the router can report the
    // reason instead of "the process exited".
    let handshake: Handshake =
        serde_json::from_str(r#"{"ready":false,"error":"module not found"}"#).unwrap();
    assert!(!handshake.ready);
    assert_eq!(handshake.error.as_deref(), Some("module not found"));
}

#[test]
fn a_request_omits_the_fields_it_does_not_set() {
    let request = JobRequest {
        id: "3".to_string(),
        code: "console.log(1)".to_string(),
        cwd: None,
        timeout_ms: None,
    };
    let line = serde_json::to_string(&request).unwrap();
    assert!(!line.contains("cwd"), "an absent cwd was still sent: {line}");
    assert!(!line.contains("timeout_ms"));
}

#[test]
fn a_response_distinguishes_a_thrown_job_from_a_broken_harness() {
    let threw: JobResponse = serde_json::from_str(
        r#"{"id":"7","ok":true,"stdout":"","stderr":"boom","exit_code":1,"elapsed_ms":12}"#,
    )
    .unwrap();
    assert!(threw.ok, "the harness ran the job; the job itself failed");
    assert_eq!(threw.exit_code, Some(1));
    assert!(threw.error.is_none());

    let broken: JobResponse =
        serde_json::from_str(r#"{"id":"7","ok":false,"error":"failed to set worker cwd"}"#).unwrap();
    assert!(!broken.ok);
    assert_eq!(broken.error.as_deref(), Some("failed to set worker cwd"));
}

#[test]
fn a_response_missing_optional_fields_still_parses() {
    // Harnesses are written in the provider's language, not this one. A field
    // one of them forgets must not take down the framing.
    let sparse: JobResponse = serde_json::from_str(r#"{"id":"1","ok":true}"#).unwrap();
    assert_eq!(sparse.stdout, "");
    assert!(!sparse.timed_out);
    assert_eq!(sparse.elapsed_ms, 0);
}
