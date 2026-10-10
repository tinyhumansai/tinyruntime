//! Existing persistent worker wire defaults and status representation.
#![allow(
    clippy::unwrap_used,
    reason = "test fixtures assert failures by panicking, matching existing suites"
)]
use super::*;

#[test]
fn legacy_worker_wire_defaults_and_status_are_preserved() {
    let ready: ReadyLine = serde_json::from_value(serde_json::json!({})).unwrap();
    assert!(!ready.ready);
    assert_eq!(ready.protocol, None);
    assert!(ready.backends.is_empty());
    let request: ServerRequest =
        serde_json::from_value(serde_json::json!({"id":"7","method":"alpha.run"})).unwrap();
    assert_eq!(request.params, serde_json::Value::Null);
    let response: ServerResponse = serde_json::from_value(serde_json::json!({"id":null})).unwrap();
    assert!(!response.ok);
    assert_eq!(response.result, None);
    assert_eq!(
        serde_json::to_value(ServerStatus::disabled("idle")).unwrap(),
        serde_json::json!({"enabled":false,"running":false,"backends":[],"message":"idle"})
    );
}
