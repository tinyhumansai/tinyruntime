//! Unit tests for the pool registry.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinyruntime_bus::{Language, PoolSettings};

use super::{Launch, Pools};

fn launch(language: Language) -> Launch {
    Launch {
        language,
        binary: "/usr/bin/node".into(),
        args: vec!["worker.js".to_string()],
        env: Vec::new(),
        protocol_version: 1,
        handshake_timeout: std::time::Duration::from_secs(30),
    }
}

#[tokio::test]
async fn the_same_launch_and_tuning_reuse_one_pool() {
    let pools = Pools::new();
    let first = pools
        .ensure(launch(Language::nodejs()), PoolSettings::default(), "22.11.0".into())
        .await;
    let second = pools
        .ensure(launch(Language::nodejs()), PoolSettings::default(), "22.11.0".into())
        .await;
    assert!(
        std::sync::Arc::ptr_eq(&first, &second),
        "the warm pool was thrown away and rebuilt"
    );
}

#[tokio::test]
async fn a_changed_interpreter_rebuilds_the_pool() {
    // Reusing warm workers across a toolchain change would answer from the old
    // interpreter, which is the one bug a fingerprint exists to prevent.
    let pools = Pools::new();
    let first = pools
        .ensure(launch(Language::nodejs()), PoolSettings::default(), "22.11.0".into())
        .await;

    let mut upgraded = launch(Language::nodejs());
    upgraded.binary = "/cache/node-v24/bin/node".into();
    let second = pools
        .ensure(upgraded, PoolSettings::default(), "24.0.0".into())
        .await;

    assert!(!std::sync::Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn retuning_the_pool_rebuilds_it() {
    let pools = Pools::new();
    let first = pools
        .ensure(launch(Language::nodejs()), PoolSettings::default(), "22.11.0".into())
        .await;
    let retuned = PoolSettings::default().with_max_workers(8);
    let second = pools
        .ensure(launch(Language::nodejs()), retuned, "22.11.0".into())
        .await;
    assert!(!std::sync::Arc::ptr_eq(&first, &second));
}

#[tokio::test]
async fn each_language_gets_its_own_pool() {
    let pools = Pools::new();
    pools
        .ensure(launch(Language::nodejs()), PoolSettings::default(), "22.11.0".into())
        .await;
    pools
        .ensure(launch(Language::python()), PoolSettings::default(), "3.12.4".into())
        .await;

    let stats = pools.stats().await;
    assert_eq!(stats.len(), 2);
    assert_eq!(stats[0].language, Language::nodejs());
    assert_eq!(stats[1].language, Language::python());
}

#[tokio::test]
async fn a_fresh_pool_reports_no_work_done() {
    let pools = Pools::new();
    pools
        .ensure(launch(Language::nodejs()), PoolSettings::default(), "22.11.0".into())
        .await;
    let stats = pools.stats().await;
    assert_eq!(stats[0].jobs_total, 0);
    assert_eq!(stats[0].worker_spawns, 0, "a pool must not spawn before it is used");
    assert_eq!(stats[0].max_workers, PoolSettings::default().effective_max_workers());
}
