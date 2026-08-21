//! Unit tests for the managed-toolchain store.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use tinyruntime_bus::Language;

use super::{InstallLock, cache_root, discard, is_inside, is_staging, promote, staging_dir};
use crate::error::Error;

#[test]
fn an_explicit_cache_directory_is_honoured_verbatim() {
    let root = cache_root(Some("/opt/runtimes"), &Language::nodejs());
    assert_eq!(root, std::path::Path::new("/opt/runtimes"));
}

#[test]
fn the_default_cache_directory_is_per_language_and_outside_any_workspace() {
    // A workspace-relative default would let a checked-out repository present a
    // directory shaped like an install and have the reuse path trust it.
    let node = cache_root(None, &Language::nodejs());
    let python = cache_root(None, &Language::python());
    assert_ne!(node, python, "two languages must not share an install root");
    assert!(node.ends_with("nodejs"));
    assert!(
        !node.starts_with("."),
        "the default must not be workspace-relative"
    );
}

#[test]
fn staging_directories_are_recognisable_and_unique() {
    let root = std::path::Path::new("/cache");
    let first = staging_dir(root);
    let second = staging_dir(root);
    assert_ne!(first, second, "two installs must not stage into one place");
    assert!(is_staging(&first));
    assert!(!is_staging(&root.join("node-v22.11.0")));
}

#[test]
fn a_symlink_out_of_the_cache_root_is_not_inside_it() {
    // The reuse path trusts what it finds under the cache root, so a link
    // planted there must not smuggle in a directory from elsewhere.
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let real = root.path().join("toolchain-1.0.0");
    fs::create_dir(&real).unwrap();
    assert!(is_inside(root.path(), &real));

    #[cfg(unix)]
    {
        let planted = root.path().join("planted");
        std::os::unix::fs::symlink(outside.path(), &planted).unwrap();
        assert!(
            !is_inside(root.path(), &planted),
            "a link out of the cache root was treated as inside it"
        );
    }
    let _ = outside;
}

#[tokio::test]
async fn promoting_installs_the_staged_directory() {
    let root = tempfile::tempdir().unwrap();
    let staged = root.path().join(".stage-1");
    fs::create_dir_all(staged.join("bin")).unwrap();
    fs::write(staged.join("bin/tool"), b"new").unwrap();

    let destination = root.path().join("toolchain-1.0.0");
    promote(&staged, &destination, &Language::nodejs())
        .await
        .expect("a staged toolchain promotes");

    assert_eq!(
        fs::read_to_string(destination.join("bin/tool")).unwrap(),
        "new"
    );
    assert!(!staged.exists(), "the staging directory was left behind");
}

#[tokio::test]
async fn promoting_over_an_existing_install_replaces_it_completely() {
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("toolchain-1.0.0");
    fs::create_dir_all(&destination).unwrap();
    fs::write(destination.join("stale"), b"old").unwrap();

    let staged = root.path().join(".stage-2");
    fs::create_dir_all(&staged).unwrap();
    fs::write(staged.join("fresh"), b"new").unwrap();

    promote(&staged, &destination, &Language::nodejs())
        .await
        .expect("a replacement promotes");

    assert!(destination.join("fresh").is_file());
    assert!(
        !destination.join("stale").is_file(),
        "the previous install's files survived the replacement"
    );
}

#[tokio::test]
async fn a_failed_promotion_leaves_the_previous_install_in_place() {
    // Losing a working toolchain to a failed upgrade is worse than the upgrade
    // failing, so the displaced install is restored rather than discarded.
    let root = tempfile::tempdir().unwrap();
    let destination = root.path().join("toolchain-1.0.0");
    fs::create_dir_all(&destination).unwrap();
    fs::write(destination.join("tool"), b"working").unwrap();

    let absent_staged = root.path().join(".stage-never-created");
    let error = promote(&absent_staged, &destination, &Language::nodejs())
        .await
        .expect_err("promoting a directory that is not there fails");

    assert!(matches!(error, Error::Install { .. }), "got {error:?}");
    assert_eq!(
        fs::read_to_string(destination.join("tool")).unwrap(),
        "working",
        "the working install was lost to a failed promotion"
    );
}

#[tokio::test]
async fn the_install_lock_is_released_when_it_is_dropped() {
    let root = tempfile::tempdir().unwrap();
    let install_dir = root.path().join("toolchain-1.0.0");

    let held = InstallLock::acquire(&install_dir, &Language::nodejs())
        .await
        .expect("the lock is taken");
    drop(held);

    // Re-acquiring would block forever if the first hold had not been released.
    let again = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        InstallLock::acquire(&install_dir, &Language::nodejs()),
    )
    .await
    .expect("re-acquiring did not block");
    assert!(again.is_ok());
}

#[tokio::test]
async fn discarding_a_missing_directory_is_not_an_error() {
    let root = tempfile::tempdir().unwrap();
    discard(&root.path().join("never-existed")).await;
}
