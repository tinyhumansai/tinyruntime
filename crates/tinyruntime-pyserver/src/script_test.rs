//! Unit tests for script placement.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use super::{Error, SCRIPT_FILE_NAME, SERVER_SCRIPT, write_script};

#[tokio::test]
async fn the_script_is_written_under_its_fixed_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("nested").join("root");
    let path = write_script(&root).await.unwrap();
    assert_eq!(path, root.join(SCRIPT_FILE_NAME));
    assert_eq!(std::fs::read_to_string(path).unwrap(), SERVER_SCRIPT);
}

#[tokio::test]
async fn the_script_is_rewritten_over_a_stale_copy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(SCRIPT_FILE_NAME), "stale").unwrap();
    let path = write_script(dir.path()).await.unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), SERVER_SCRIPT);
}

#[tokio::test]
async fn a_root_that_is_a_file_cannot_be_created() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("occupied");
    std::fs::write(&file, "x").unwrap();
    let error = write_script(&file.join("child")).await.unwrap_err();
    assert!(matches!(error, Error::CreateDir { .. }), "{error}");
}

#[tokio::test]
async fn a_script_path_that_is_a_directory_cannot_be_written() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(SCRIPT_FILE_NAME)).unwrap();
    let error = write_script(dir.path()).await.unwrap_err();
    assert!(matches!(error, Error::WriteScript { .. }), "{error}");
}

#[test]
fn the_embedded_script_speaks_the_crate_protocol() {
    assert!(SERVER_SCRIPT.contains(&format!("PROTOCOL = {}", crate::PROTOCOL_VERSION)));
}
