//! Capability-relative writes refuse links and stay inside opened directories.
#![allow(
    clippy::unwrap_used,
    reason = "local fixture assertions match existing test conventions"
)]
use super::*;

#[cfg(unix)]
#[test]
fn parent_symlink_replacement_cannot_redirect_router_publication() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let dir = absolute(&root.path().canonicalize().unwrap(), true).unwrap();
    std::fs::create_dir(root.path().join("target")).unwrap();
    let (pinned, _) = parent(&dir, "target/ready", false).unwrap();
    std::fs::rename(root.path().join("target"), root.path().join("old")).unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("target")).unwrap();
    assert!(parent(&dir, "target/ready", true).is_err());
    stage(&pinned, "ready", b"owned").unwrap();
    assert!(!outside.path().join("ready").exists());
    assert_eq!(
        std::fs::read(root.path().join("old/ready")).unwrap(),
        b"owned"
    );
}

#[cfg(unix)]
#[test]
fn named_pipe_artifact_is_rejected_without_waiting_for_a_writer() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(root.path().join("marker"))
            .status()
            .unwrap()
            .success()
    );
    let dir = absolute(&root.path().canonicalize().unwrap(), false).unwrap();
    let artifact = CacheArtifact {
        path: "marker".into(),
        bytes: Vec::new(),
    };
    assert_eq!(matches(&dir, &artifact).unwrap_err(), "cache_entry_unsafe");
}

#[test]
fn bounded_regular_artifacts_and_required_entries_preserve_exact_bytes() {
    let root = tempfile::tempdir().unwrap();
    let dir = absolute(&root.path().canonicalize().unwrap(), true).unwrap();
    let artifact = CacheArtifact {
        path: "nested/file".into(),
        bytes: b"payload".to_vec(),
    };
    assert!(!matches(&dir, &artifact).unwrap());
    let (nested, name) = parent(&dir, &artifact.path, true).unwrap();
    stage(&nested, &name, &artifact.bytes).unwrap();
    assert!(matches(&dir, &artifact).unwrap());
    assert!(stage(&nested, &name, b"duplicate").is_err());
    assert!(
        required(
            &dir,
            &CacheEntry {
                path: "nested".into(),
                kind: CacheEntryKind::Directory
            }
        )
        .unwrap()
    );
    assert!(
        required(
            &dir,
            &CacheEntry {
                path: "nested/file".into(),
                kind: CacheEntryKind::File
            }
        )
        .unwrap()
    );
    assert!(
        !required(
            &dir,
            &CacheEntry {
                path: "missing/file".into(),
                kind: CacheEntryKind::Entry
            }
        )
        .unwrap()
    );
    assert!(
        !required(
            &dir,
            &CacheEntry {
                path: "missing".into(),
                kind: CacheEntryKind::File
            }
        )
        .unwrap()
    );
    std::fs::write(root.path().join("nested/file"), vec![1; MAX_FILE + 1]).unwrap();
    assert!(!matches(&dir, &artifact).unwrap());
    invalidate(&dir, "nested/file").unwrap();
    invalidate(&dir, "nested/file").unwrap();
    invalidate(&dir, "missing/file").unwrap();
    assert!(parent(&dir, "../bad", true).is_err());
    assert!(promote(&dir, "missing", &dir, "nested/new").is_err());
}

#[cfg(unix)]
#[test]
fn final_symlinks_and_hardlinks_are_never_read_or_overwritten_as_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), b"private").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    std::fs::hard_link(outside.path().join("secret"), root.path().join("hard")).unwrap();
    let dir = absolute(&root.path().canonicalize().unwrap(), false).unwrap();
    for path in ["link", "hard"] {
        assert!(
            matches(
                &dir,
                &CacheArtifact {
                    path: path.into(),
                    bytes: b"private".to_vec()
                }
            )
            .is_err()
        );
        assert!(invalidate(&dir, path).is_err());
        assert!(promote(&dir, "missing", &dir, path).is_err());
    }
    assert!(
        required(
            &dir,
            &CacheEntry {
                path: "link".into(),
                kind: CacheEntryKind::Entry
            }
        )
        .unwrap()
    );
    assert!(
        !required(
            &dir,
            &CacheEntry {
                path: "link".into(),
                kind: CacheEntryKind::File
            }
        )
        .unwrap()
    );
    assert_eq!(
        std::fs::read(outside.path().join("secret")).unwrap(),
        b"private"
    );
    assert!(absolute(&root.path().join("link"), false).is_err());
}
