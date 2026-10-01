//! Unit tests for the worker wire types.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{PROTOCOL_VERSION, ReadyLine, ServerRequest, ServerResponse};

#[test]
fn ready_line_parses() {
    let ready: ReadyLine =
        serde_json::from_str(r#"{"ready":true,"protocol":1,"backends":["spacy"]}"#).unwrap();
    assert!(ready.ready);
    assert_eq!(ready.protocol, Some(PROTOCOL_VERSION));
    assert_eq!(ready.backends, vec!["spacy"]);
}

#[test]
fn a_bare_ready_line_defaults_to_not_ready() {
    let ready: ReadyLine = serde_json::from_str("{}").unwrap();
    assert!(!ready.ready);
    assert_eq!(ready.protocol, None);
    assert_eq!(ready.backends.len(), 0);
    assert_eq!(ready.error, None);
}

#[test]
fn response_parses_error_envelope() {
    let response: ServerResponse = serde_json::from_str(
        r#"{"id":"7","ok":false,"error":{"code":"bad_request","message":"missing text"}}"#,
    )
    .unwrap();
    assert!(!response.ok);
    assert_eq!(response.id.as_deref(), Some("7"));
    assert_eq!(response.error.unwrap().code, "bad_request");
}

#[test]
fn a_request_serializes_to_the_documented_line() {
    // The worker script parses exactly this shape; the field names are the
    // wire contract.
    let request = ServerRequest {
        id: "3".to_string(),
        method: "spacy.extract".to_string(),
        params: serde_json::json!({ "text": "hi" }),
    };
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"id":"3","method":"spacy.extract","params":{"text":"hi"}}"#
    );
}

#[test]
fn a_request_without_params_defaults_to_null() {
    let request: ServerRequest = serde_json::from_str(r#"{"id":"1","method":"m"}"#).unwrap();
    assert!(request.params.is_null());
}
