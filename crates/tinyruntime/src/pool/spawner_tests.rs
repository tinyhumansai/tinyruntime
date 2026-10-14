//! The spawner seam: workers launch through it, and a refusal fails closed.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tinyruntime_bus::Language;

use super::{SpawnRefused, SpawnSpec, Spawner};
use crate::pool::fake_worker;
use crate::pool::worker::Worker;

/// Counts calls and optionally wraps the program, like a sandbox launcher.
#[derive(Debug)]
struct Recording {
    calls: AtomicUsize,
    refuse: bool,
    id: &'static str,
}

impl Recording {
    fn new(refuse: bool, id: &'static str) -> Arc<Self> {
        Arc::new(Self {
            calls: AtomicUsize::new(0),
            refuse,
            id,
        })
    }
}

impl Spawner for Recording {
    fn id(&self) -> &str {
        self.id
    }

    fn confine(&self, spec: SpawnSpec) -> Result<SpawnSpec, SpawnRefused> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.refuse {
            return Err(SpawnRefused("policy denies this launch".to_string()));
        }
        // Wrap the real program the way a sandbox launcher would: the child the
        // OS starts is `env`, which then execs the original program.
        let mut args = vec![spec.program.display().to_string()];
        args.extend(spec.args);
        Ok(SpawnSpec {
            program: "/usr/bin/env".into(),
            args,
            ..spec
        })
    }
}

#[test]
fn without_a_spawner_the_spec_is_unchanged() {
    let spec = SpawnSpec {
        program: "node".into(),
        args: vec!["w.js".to_string()],
        env: Vec::new(),
        clear_env: true,
    };
    assert_eq!(super::confine(None, spec.clone()).unwrap(), spec);
}

#[test]
fn the_launch_fingerprint_names_the_spawner() {
    let direct = fake_worker::launch(Language::nodejs());
    let confined = fake_worker::launch(Language::nodejs()).with_spawner(Recording::new(false, "a"));
    let other = fake_worker::launch(Language::nodejs()).with_spawner(Recording::new(false, "b"));
    assert_ne!(direct.fingerprint(), confined.fingerprint());
    assert_ne!(confined.fingerprint(), other.fingerprint());
}

#[cfg(unix)]
#[tokio::test]
async fn a_worker_is_started_through_the_host_spawner() {
    let spawner = Recording::new(false, "wrap");
    let launch = fake_worker::launch(Language::nodejs()).with_spawner(spawner.clone());
    let worker = Worker::spawn(&launch).await.expect("worker starts");
    assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
    worker.shutdown();
}

#[cfg(unix)]
#[tokio::test]
async fn a_refusing_spawner_starts_nothing_and_does_not_fall_back() {
    let spawner = Recording::new(true, "deny");
    let launch = fake_worker::launch(Language::nodejs()).with_spawner(spawner.clone());
    let error = Worker::spawn(&launch).await.expect_err("refused");
    assert!(error.contains("refused"), "got `{error}`");
    assert_eq!(spawner.calls.load(Ordering::SeqCst), 1);
}
