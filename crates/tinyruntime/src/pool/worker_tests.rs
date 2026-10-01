//! Unit tests for worker launch and framing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use tinyruntime_bus::Language;

use super::{Launch, SubmitFailure, verify_handshake};
use crate::pool::protocol::Handshake;

fn launch() -> Launch {
    Launch {
        language: Language::nodejs(),
        binary: "/usr/bin/node".into(),
        args: vec!["worker.js".to_string()],
        env: vec![("PATH".to_string(), "/usr/bin".to_string())],
        protocol_version: 1,
        handshake_timeout: Duration::from_secs(30),
    }
}

#[test]
fn a_fingerprint_changes_with_anything_that_changes_the_toolchain() {
    let base = launch();
    assert_eq!(base.fingerprint(), launch().fingerprint());

    let mut other_binary = launch();
    other_binary.binary = "/opt/node/bin/node".into();
    assert_ne!(base.fingerprint(), other_binary.fingerprint());

    let mut other_flags = launch();
    other_flags.args.push("--flag".to_string());
    assert_ne!(base.fingerprint(), other_flags.fingerprint());

    let mut other_env = launch();
    other_env.env.push(("EXTRA".to_string(), "1".to_string()));
    assert_ne!(
        base.fingerprint(),
        other_env.fingerprint(),
        "a changed environment must not be served by the old warm workers"
    );
}

#[test]
fn a_handshake_without_the_secret_is_refused() {
    // Anything on the machine can connect to a loopback port; only the child the
    // router spawned was told the secret.
    let handshake = Handshake {
        ready: true,
        protocol: Some(1),
        token: Some("guessed".to_string()),
        ..Handshake::default()
    };
    let error = verify_handshake(&handshake, &launch(), "issued").expect_err("refused");
    assert!(error.contains("secret"), "got `{error}`");
}

#[test]
fn a_handshake_on_another_protocol_is_refused() {
    let handshake = Handshake {
        ready: true,
        protocol: Some(2),
        token: Some("issued".to_string()),
        ..Handshake::default()
    };
    let error = verify_handshake(&handshake, &launch(), "issued").expect_err("refused");
    assert!(error.contains("protocol 2"), "got `{error}`");
}

#[test]
fn a_handshake_that_names_no_protocol_is_refused() {
    // A harness that omits the field must not be read as agreeing with us.
    let handshake = Handshake {
        ready: true,
        protocol: None,
        token: Some("issued".to_string()),
        ..Handshake::default()
    };
    let error = verify_handshake(&handshake, &launch(), "issued").expect_err("refused");
    assert!(error.contains("which protocol"), "got `{error}`");
}

#[test]
fn the_handshake_budget_does_not_take_part_in_the_fingerprint() {
    // It changes how long a failing spawn takes, not which toolchain the warm
    // workers are running; rebuilding a healthy pool over it would be waste.
    let mut impatient = launch();
    impatient.handshake_timeout = Duration::from_millis(1);
    assert_eq!(launch().fingerprint(), impatient.fingerprint());
}

#[test]
fn a_worker_that_failed_to_start_reports_its_own_reason() {
    let handshake = Handshake {
        ready: false,
        error: Some("module not found".to_string()),
        ..Handshake::default()
    };
    let error = verify_handshake(&handshake, &launch(), "issued").expect_err("refused");
    assert!(error.contains("module not found"), "got `{error}`");
}

#[test]
fn a_good_handshake_is_accepted() {
    let handshake = Handshake {
        ready: true,
        protocol: Some(1),
        token: Some("issued".to_string()),
        ..Handshake::default()
    };
    assert!(verify_handshake(&handshake, &launch(), "issued").is_ok());
}

#[test]
fn dispatch_tagging_is_what_keeps_a_job_from_running_twice() {
    assert!(!SubmitFailure::pre("write failed").dispatched);
    assert!(SubmitFailure::post("read wedged").dispatched);
}

#[tokio::test]
async fn a_worker_that_never_connects_back_fails_rather_than_hanging() {
    // `true` exits immediately without ever opening the protocol connection.
    let binary = if cfg!(windows) { "cmd" } else { "/bin/true" };
    if !std::path::Path::new(binary).exists() && !cfg!(windows) {
        return;
    }
    let mut spec = launch();
    spec.handshake_timeout = Duration::from_secs(2);
    spec.binary = binary.into();
    spec.args = if cfg!(windows) {
        vec!["/c".to_string(), "exit".to_string()]
    } else {
        Vec::new()
    };

    let outcome = tokio::time::timeout(
        Duration::from_secs(30),
        crate::pool::worker::Worker::spawn(&spec),
    )
    .await
    .expect("the spawn attempt finished");
    assert!(outcome.is_err(), "a worker that never connects must fail");
}
