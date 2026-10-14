//! The spawner seam: the worker starts through it, and a refusal fails closed.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Value, json};

use super::{SpawnRefused, SpawnSpec, Spawner};
use crate::PythonServer;
use crate::error::Error;
use crate::testing::healthy;

#[derive(Debug)]
struct Gate {
    refuse: bool,
    calls: AtomicUsize,
}

impl Spawner for Gate {
    fn id(&self) -> &str {
        "gate"
    }

    fn confine(&self, spec: SpawnSpec) -> Result<SpawnSpec, SpawnRefused> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.refuse {
            return Err(SpawnRefused("policy denies python".to_string()));
        }
        // Wrap like a sandbox launcher: `env` execs the real interpreter.
        let mut args = vec![spec.program.display().to_string()];
        args.extend(spec.args);
        Ok(SpawnSpec {
            program: "/usr/bin/env".into(),
            args,
            ..spec
        })
    }
}

#[tokio::test]
async fn the_worker_is_started_through_the_host_spawner() {
    let dir = tempfile::tempdir().unwrap();
    let gate = Arc::new(Gate {
        refuse: false,
        calls: AtomicUsize::new(0),
    });
    let server = PythonServer::new(healthy(dir.path()).with_spawner(gate.clone()));
    let value: Value = server.request("alpha.echo", json!({})).await.unwrap();
    assert_eq!(value["n"], 1);
    assert_eq!(gate.calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn a_refusing_spawner_starts_nothing_and_does_not_fall_back() {
    let dir = tempfile::tempdir().unwrap();
    let gate = Arc::new(Gate {
        refuse: true,
        calls: AtomicUsize::new(0),
    });
    let server = PythonServer::new(healthy(dir.path()).with_spawner(gate.clone()));
    let error = server
        .request::<Value>("alpha.echo", json!({}))
        .await
        .expect_err("refused");
    assert!(matches!(error, Error::SpawnRefused(_)), "got {error}");
    assert!(!server.status().await.running);
    assert!(!dir.path().join("mark").exists());
}

#[test]
fn without_a_spawner_the_spec_is_unchanged() {
    let spec = SpawnSpec {
        program: "python3".into(),
        args: vec!["-u".to_string()],
        env: Vec::new(),
        clear_env: false,
    };
    assert_eq!(super::confine(None, spec.clone()).unwrap(), spec);
}
