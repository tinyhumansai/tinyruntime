//! Unit tests for a single worker, run against shell-script fakes.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use serde_json::{Value, json};

use super::PythonServer;
use crate::error::Error;
use crate::testing::{ECHO_LOOP, READY, healthy, launch};

#[tokio::test]
async fn a_request_round_trips_and_status_reports_the_backend_ready() {
    let dir = tempfile::tempdir().unwrap();
    let server = PythonServer::new(healthy(dir.path()));
    assert_eq!(server.backends(), ["alpha".to_string()]);

    let before = server.status().await;
    assert!(before.enabled && !before.running);
    assert!(!before.backends[0].ready);

    server.start().await.unwrap();
    // Starting twice is a no-op, not a second child.
    server.start().await.unwrap();
    let value: Value = server.request("alpha.echo", json!({})).await.unwrap();
    assert_eq!(value, json!({ "n": 1 }));

    let after = server.status().await;
    assert!(after.running);
    assert!(after.backends[0].ready);
    assert!(!server.idle_expired(Duration::from_secs(3600)).await);
    assert!(server.idle_expired(Duration::ZERO).await);
}

#[tokio::test]
async fn the_first_request_spawns_the_worker_on_demand() {
    let dir = tempfile::tempdir().unwrap();
    let server = PythonServer::new(healthy(dir.path()));
    let value: Value = server.request("alpha.echo", json!({})).await.unwrap();
    assert_eq!(value["n"], 1);
    server.reset().await;
    assert!(!server.status().await.running);
    // A reset worker comes back on the next request.
    let again: Value = server.request("alpha.echo", json!({})).await.unwrap();
    assert_eq!(again["n"], 1);
}

#[tokio::test]
async fn lines_that_are_not_this_requests_response_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        r#"{READY}
while IFS= read -r line; do
  id=${{line#*\"id\":\"}}; id=${{id%%\"*}}
  printf 'not json\n'
  printf '{{"id":"stale","ok":true,"result":{{"n":99}}}}\n'
  printf '{{"id":"%s","ok":true,"result":{{"n":1}}}}\n' "$id"
done
"#
    );
    let server = PythonServer::new(launch(dir.path(), &body));
    let value: Value = server.request("alpha.echo", json!({})).await.unwrap();
    assert_eq!(value["n"], 1);
}

#[tokio::test]
async fn a_worker_reported_failure_names_the_method_and_code() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        r#"{READY}
while IFS= read -r line; do
  id=${{line#*\"id\":\"}}; id=${{id%%\"*}}
  printf '{{"id":"%s","ok":false,"error":{{"code":"bad","message":"nope"}}}}\n' "$id"
done
"#
    );
    let server = PythonServer::new(launch(dir.path(), &body));
    let error = server
        .request::<Value>("alpha.x", json!({}))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "runtime python server `alpha.x` failed: bad: nope"
    );
}

#[tokio::test]
async fn a_failure_without_detail_is_reported_as_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!(
        r#"{READY}
while IFS= read -r line; do
  id=${{line#*\"id\":\"}}; id=${{id%%\"*}}
  printf '{{"id":"%s","ok":false}}\n' "$id"
done
"#
    );
    let server = PythonServer::new(launch(dir.path(), &body));
    let error = server
        .request::<Value>("alpha.x", json!({}))
        .await
        .unwrap_err();
    assert!(
        error.to_string().ends_with("unknown python server error"),
        "{error}"
    );
}

#[tokio::test]
async fn a_result_of_the_wrong_shape_is_a_decode_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = PythonServer::new(healthy(dir.path()));
    let error = server
        .request::<String>("alpha.echo", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Decode { .. }), "{error}");
}

#[tokio::test]
async fn a_crashed_worker_is_restarted_and_the_request_retried_once() {
    let dir = tempfile::tempdir().unwrap();
    // The first process answers the handshake, takes one request, and dies. The
    // marker file makes the second one healthy.
    let body = format!(
        r#"{READY}
if [ ! -e "$MARK" ]; then
  : > "$MARK"
  IFS= read -r line
  exit 0
fi
{ECHO_LOOP}"#
    );
    let server = PythonServer::new(launch(dir.path(), &body));
    let value: Value = server.request("alpha.echo", json!({})).await.unwrap();
    assert_eq!(value["n"], 1);
}

#[tokio::test]
async fn a_worker_that_never_answers_times_out() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{READY}\nIFS= read -r line\nsleep 30\n");
    let mut spec = launch(dir.path(), &body);
    spec.request_timeout = Duration::from_millis(150);
    let server = PythonServer::new(spec);
    let error = server
        .request::<Value>("alpha.x", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::RequestTimeout), "{error}");
}

#[tokio::test]
async fn an_unreadable_response_line_is_a_read_error() {
    let dir = tempfile::tempdir().unwrap();
    let body = format!("{READY}\nIFS= read -r line\nprintf '\\377\\n'\nsleep 30\n");
    let server = PythonServer::new(launch(dir.path(), &body));
    let error = server
        .request::<Value>("alpha.x", json!({}))
        .await
        .unwrap_err();
    assert!(matches!(error, Error::Read(_)), "{error}");
}

#[tokio::test]
async fn a_worker_that_exits_before_ready_fails_the_start() {
    let dir = tempfile::tempdir().unwrap();
    let server = PythonServer::new(launch(dir.path(), "exit 0\n"));
    let error = server.start().await.unwrap_err();
    assert!(matches!(error, Error::ExitedBeforeReady), "{error}");
}

#[tokio::test]
async fn a_worker_that_reports_failure_carries_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"printf '%s\n' '{"ready":false,"error":"boom"}'"#;
    let server = PythonServer::new(launch(dir.path(), body));
    let error = server.start().await.unwrap_err();
    assert_eq!(
        error.to_string(),
        "runtime python server failed to start: boom"
    );

    let body = r#"printf '%s\n' '{"ready":false}'"#;
    let server = PythonServer::new(launch(dir.path(), body));
    let error = server.start().await.unwrap_err();
    assert!(error.to_string().ends_with(": unknown"), "{error}");
}

#[tokio::test]
async fn a_protocol_mismatch_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let body = r#"printf '%s\n' '{"ready":true,"protocol":99,"backends":[]}'"#;
    let server = PythonServer::new(launch(dir.path(), body));
    let error = server.start().await.unwrap_err();
    assert!(
        matches!(
            error,
            Error::ProtocolMismatch {
                expected: 1,
                got: Some(99)
            }
        ),
        "{error}"
    );
}

#[tokio::test]
async fn an_unparseable_ready_line_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let server = PythonServer::new(launch(dir.path(), "echo hello\n"));
    let error = server.start().await.unwrap_err();
    assert!(matches!(error, Error::ReadyParse { .. }), "{error}");
}

#[tokio::test]
async fn an_unreadable_ready_line_is_a_handshake_read_error() {
    let dir = tempfile::tempdir().unwrap();
    let server = PythonServer::new(launch(dir.path(), "printf '\\377\\n'\nsleep 30\n"));
    let error = server.start().await.unwrap_err();
    assert!(matches!(error, Error::HandshakeRead(_)), "{error}");
}

#[tokio::test]
async fn a_silent_worker_times_out_the_handshake() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = launch(dir.path(), "sleep 30\n");
    spec.handshake_timeout = Duration::from_millis(150);
    let server = PythonServer::new(spec);
    let error = server.start().await.unwrap_err();
    assert!(matches!(error, Error::HandshakeTimeout), "{error}");
}

#[tokio::test]
async fn a_missing_interpreter_is_a_spawn_error_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut spec = healthy(dir.path());
    spec.python_bin = dir.path().join("no-such-python");
    let server = PythonServer::new(spec);
    let error = server.start().await.unwrap_err();
    assert!(
        matches!(&error, Error::Spawn { bin, .. } if bin.ends_with("no-such-python")),
        "{error}"
    );
}
