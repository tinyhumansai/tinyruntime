//! Unit tests for the status types.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{BackendStatus, ServerStatus};

#[test]
fn a_disabled_status_carries_its_reason_and_no_backends() {
    let status = ServerStatus::disabled("has not started");
    assert!(!status.enabled);
    assert!(!status.running);
    assert!(status.backends.is_empty());
    assert_eq!(status.message.as_deref(), Some("has not started"));
}

#[test]
fn status_serializes_with_stable_field_names() {
    let status = ServerStatus {
        enabled: true,
        running: true,
        backends: vec![BackendStatus {
            id: "spacy".to_string(),
            enabled: true,
            ready: true,
            message: None,
        }],
        message: None,
    };
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"enabled":true,"running":true,"backends":[{"id":"spacy","enabled":true,"ready":true,"message":null}],"message":null}"#
    );
}
