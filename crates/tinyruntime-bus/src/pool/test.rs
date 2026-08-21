//! Unit tests for the pool payloads.

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
