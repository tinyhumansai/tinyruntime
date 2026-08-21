//! Unit tests for the pool payloads.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::PoolSettings;

#[test]
fn the_default_pool_is_small_and_recycles() {
    let settings = PoolSettings::default();
    assert!(settings.enabled);
    assert_eq!(settings.effective_max_workers(), 2);
    assert_eq!(settings.recycle_after_jobs, 100);
}

#[test]
fn a_zero_worker_pool_is_clamped_rather_than_deadlocking() {
    let settings = PoolSettings {
        max_workers: 0,
        ..PoolSettings::default()
    };
    assert_eq!(settings.effective_max_workers(), 1);
}

#[test]
fn a_zero_ttl_keeps_workers_warm_forever() {
    let settings = PoolSettings {
        idle_ttl_secs: 0,
        ..PoolSettings::default()
    };
    assert_eq!(settings.idle_ttl_secs(), None);
    assert_eq!(PoolSettings::default().idle_ttl_secs(), Some(300));
}

#[test]
fn a_zero_queue_depth_sheds_immediately_rather_than_being_clamped_up() {
    let settings = PoolSettings {
        max_queue_depth: 0,
        ..PoolSettings::default()
    };
    assert_eq!(settings.effective_max_queue_depth(), 0);
}

#[test]
fn pins_its_wire_representation() {
    let value = serde_json::to_value(PoolSettings::default()).expect("pool settings serialise");
    assert_eq!(
        value,
        serde_json::json!({
            "enabled": true,
            "max_workers": 2,
            "idle_ttl_secs": 300,
            "recycle_after_jobs": 100,
            "max_queue_depth": 256,
        })
    );
}

#[test]
fn every_knob_has_a_builder() {
    let settings = PoolSettings::default()
        .with_enabled(false)
        .with_max_workers(7)
        .with_idle_ttl_secs(42)
        .with_recycle_after_jobs(9)
        .with_max_queue_depth(11);

    assert!(!settings.enabled);
    assert_eq!(settings.effective_max_workers(), 7);
    assert_eq!(settings.idle_ttl_secs(), Some(42));
    assert_eq!(settings.recycle_after_jobs, 9);
    assert_eq!(settings.effective_max_queue_depth(), 11);
}

#[test]
fn stats_start_at_zero_and_record_what_a_pool_served() {
    let stats = super::PoolStats::new(crate::Language::nodejs(), 4);
    assert_eq!(stats.jobs_total, 0);
    assert_eq!(stats.max_workers, 4);

    let recorded = stats.with_counts(10, 2, 1).with_idle_workers(3);
    assert_eq!(recorded.jobs_total, 10);
    assert_eq!(recorded.worker_spawns, 2);
    assert_eq!(recorded.rejected_saturated, 1);
    assert_eq!(recorded.idle_workers, 3);
}

#[test]
fn a_stats_reply_carries_the_pools_it_was_built_from() {
    let response = super::PoolStatsResponse::new(vec![super::PoolStats::new(
        crate::Language::python(),
        2,
    )]);
    assert_eq!(response.pools.len(), 1);
    assert_eq!(response.pools[0].language, crate::Language::python());
}
