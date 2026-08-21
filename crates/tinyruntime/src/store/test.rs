//! Unit tests for the managed-toolchain store.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;

use tinyruntime_bus::Language;

use super::{
    InstallLock, cache_root, discard, ensure_root, is_inside, is_staging, promote, staging_dir,
};
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

#[test]
fn a_language_with_no_platform_cache_still_gets_a_root() {
    // Only reachable where `dirs` has no answer, but the fallback must be a real
    // path rather than an empty one — an install into "" would land anywhere.
    let root = cache_root(None, &Language::new("ruby"));
    assert!(root.iter().count() >= 2, "got {}", root.display());
    assert!(root.ends_with("ruby"));
}

#[test]
fn a_path_that_is_not_there_is_not_inside_anything() {
    // `is_inside` canonicalises, which fails for a path that does not exist.
    // Answering "yes" would let the reuse scan trust a directory that vanished.
    let root = tempfile::tempdir().unwrap();
    assert!(!is_inside(root.path(), &root.path().join("absent")));
    assert!(!is_inside(std::path::Path::new("/absent-root"), root.path()));
}

#[test]
fn a_sibling_directory_is_not_inside_the_cache_root() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    assert!(!is_inside(first.path(), second.path()));
}

#[tokio::test]
async fn ensuring_a_root_creates_every_missing_level() {
    let scratch = tempfile::tempdir().unwrap();
    let nested = scratch.path().join("a/b/c");
    ensure_root(&nested).await.expect("the root is created");
    assert!(nested.is_dir());
    // Idempotent: an existing root is not an error.
    ensure_root(&nested).await.expect("an existing root is fine");
}

#[tokio::test]
async fn ensuring_a_root_under_a_file_reports_a_storage_failure() {
    let scratch = tempfile::tempdir().unwrap();
    let blocker = scratch.path().join("not-a-directory");
    fs::write(&blocker, b"x").unwrap();

    let error = ensure_root(&blocker.join("child"))
        .await
        .expect_err("a file cannot contain a directory");
    assert!(matches!(error, Error::Storage(_)), "got {error:?}");
}

#[tokio::test]
async fn promoting_creates_the_cache_root_when_it_is_missing() {
    let scratch = tempfile::tempdir().unwrap();
    let staged = scratch.path().join("staged");
    fs::create_dir_all(&staged).unwrap();
    fs::write(staged.join("tool"), b"x").unwrap();

    let destination = scratch.path().join("nested/root/toolchain-1.0.0");
    promote(&staged, &destination, &Language::nodejs())
        .await
        .expect("the parent is created on the way");
    assert!(destination.join("tool").is_file());
}

#[tokio::test]
async fn a_second_holder_waits_for_the_first_to_release() {
    // Two processes deciding to install the same toolchain at the same moment is
    // the case this lock exists for: the second must wait, not proceed.
    let root = tempfile::tempdir().unwrap();
    let install_dir = root.path().join("toolchain-1.0.0");

    let held = InstallLock::acquire(&install_dir, &Language::nodejs())
        .await
        .expect("the first holder takes it");

    let contender = install_dir.clone();
    let waiting = tokio::spawn(async move {
        InstallLock::acquire(&contender, &Language::nodejs()).await
    });

    // The second acquire must still be blocked while the first is held.
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert!(!waiting.is_finished(), "the lock did not exclude a second holder");

    drop(held);
    let second = tokio::time::timeout(std::time::Duration::from_secs(5), waiting)
        .await
        .expect("the waiter is released once the lock is dropped")
        .expect("the task completed");
    assert!(second.is_ok());
}

#[tokio::test]
async fn a_lock_under_an_unusable_parent_reports_an_install_failure() {
    let scratch = tempfile::tempdir().unwrap();
    let blocker = scratch.path().join("not-a-directory");
    fs::write(&blocker, b"x").unwrap();

    let error = InstallLock::acquire(&blocker.join("child/toolchain"), &Language::nodejs())
        .await
        .expect_err("a file cannot contain a lock");
    assert!(
        matches!(error, Error::Storage(_) | Error::Install { .. }),
        "got {error:?}"
    );
}
