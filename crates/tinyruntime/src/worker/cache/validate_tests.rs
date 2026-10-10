//! Invalid inputs are rejected before scoped cache I/O.
#![allow(
    clippy::unwrap_used,
    reason = "local fixture assertions match existing test conventions"
)]
use super::*;

#[test]
fn portable_relative_paths_reject_traversal_credentials_and_aliases() {
    for path in [
        "",
        "/root",
        "a//b",
        "a/../b",
        ".",
        ".ssh/key",
        "a\\b",
        "a:b",
        "a\0b",
        "a\nb",
        "a.",
        "a ",
        "CON.txt",
        "lpt1",
        ".tinyruntime-cache.lock",
        ".tinyruntime-stage-x",
        ".aws/token",
    ] {
        assert!(relative(path).is_err(), "{path:?}");
    }
    assert!(relative("dir/.ready-v1").is_ok());
    assert!(relative(&"x".repeat(513)).is_err());
}

#[test]
fn scopes_refuse_ambient_system_and_credential_roots() {
    for root in [
        "relative",
        "/",
        "/tmp",
        "/tmp/../cache",
        "/tmp/.gnupg/data",
        "/etc/private",
        "/tmp/cache\0bad",
    ] {
        assert!(
            scope(&WorkerCacheScope {
                id: "approved".into(),
                root: root.into()
            })
            .is_err()
        );
    }
    assert!(
        scope(&WorkerCacheScope {
            id: "bad/id".into(),
            root: "/tmp/cache".into()
        })
        .is_err()
    );
    if let Some(home) = dirs::home_dir() {
        assert!(
            scope(&WorkerCacheScope {
                id: "ok".into(),
                root: home.to_string_lossy().into_owned()
            })
            .is_err()
        );
    }
}

fn valid() -> CacheRecipe {
    CacheRecipe {
        scope: "ok".into(),
        artifacts: Vec::new(),
        steps: Vec::new(),
        required: Vec::new(),
        marker: tinyruntime_bus::worker::CacheArtifact {
            path: "ready".into(),
            bytes: Vec::new(),
        },
        adoption: tinyruntime_bus::worker::CacheAdoptionPolicy::Strict,
        timeout_ms: 1000,
    }
}

#[test]
fn recipe_bounds_are_checked_before_native_or_filesystem_expansion() {
    let initial = valid();
    assert!(recipe(&initial).is_ok());
    for mutation in 0..10 {
        let mut value = initial.clone();
        match mutation {
            0 => value.scope.clear(),
            1 => value.timeout_ms = 0,
            2 => value.timeout_ms = 4 * 60 * 60 * 1000 + 1,
            3 => value.marker.path = "../ready".into(),
            4 => value.marker.bytes = vec![0; MAX_FILE + 1],
            5 => value.artifacts = vec![value.marker.clone()],
            6 => {
                value.required = vec![tinyruntime_bus::worker::CacheEntry {
                    path: "../bad".into(),
                    kind: tinyruntime_bus::worker::CacheEntryKind::File,
                }];
            }
            7 => {
                value.steps = vec![tinyruntime_bus::worker::WorkerCommand {
                    executable: "implicit".into(),
                    args: Vec::new(),
                    env: Vec::new(),
                    timeout_ms: None,
                }];
            }
            8 => {
                value.steps = vec![tinyruntime_bus::worker::WorkerCommand {
                    executable: "/runtime".into(),
                    args: Vec::new(),
                    env: Vec::new(),
                    timeout_ms: Some(30 * 60 * 1000 + 1),
                }];
            }
            _ => {
                value.artifacts = (0..17)
                    .map(|i| tinyruntime_bus::worker::CacheArtifact {
                        path: format!("file-{i}"),
                        bytes: Vec::new(),
                    })
                    .collect();
            }
        }
        assert!(recipe(&value).is_err(), "mutation {mutation}");
    }
    let mut value = initial;
    value.artifacts = (0..16)
        .map(|i| tinyruntime_bus::worker::CacheArtifact {
            path: format!("file-{i}"),
            bytes: vec![0; MAX_FILE],
        })
        .collect();
    value.marker.bytes = vec![1];
    assert!(recipe(&value).is_err());
}

#[test]
fn the_whole_deadline_can_cover_every_legacy_provisioning_step() {
    let mut value = valid();
    value.timeout_ms = 7_320_000;
    assert!(recipe(&value).is_ok());
}
