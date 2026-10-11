//! Native spawning preserves directory identity through namespace replacement.
use super::*;
use std::fs::File;
use std::process::Command;

#[test]
fn child_uses_owned_directory_after_rename_and_symlink_replacement() {
    let scratch = tempfile::tempdir().unwrap();
    let root = scratch.path().join("root");
    let held = scratch.path().join("held");
    let outside = scratch.path().join("outside");
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(root.join("marker"), "pinned").unwrap();
    std::fs::write(outside.join("marker"), "replacement").unwrap();
    let directory = File::open(&root).unwrap();
    let parent_cwd = std::env::current_dir().unwrap();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "cat marker"]);
    current_dir(&mut command, &directory).unwrap();
    drop(directory);
    std::fs::rename(&root, &held).unwrap();
    std::os::unix::fs::symlink(&outside, &root).unwrap();
    let output = command.output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"pinned");
    // A reusable command retains the same owned fd for every spawn.
    assert_eq!(command.output().unwrap().stdout, b"pinned");
    assert_eq!(std::env::current_dir().unwrap(), parent_cwd);
}

#[test]
fn spawning_with_a_file_as_cwd_fails_without_changing_parent_directory() {
    let file = tempfile::tempfile().unwrap();
    let parent_cwd = std::env::current_dir().unwrap();
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    current_dir(&mut command, &file).unwrap();
    assert_eq!(
        command.spawn().unwrap_err().kind(),
        std::io::ErrorKind::NotADirectory
    );
    assert_eq!(std::env::current_dir().unwrap(), parent_cwd);
}
