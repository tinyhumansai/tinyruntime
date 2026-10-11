//! Native spawning preserves directory identity through namespace replacement.
use super::*;
use std::fs::File;
use std::process::Command;

#[test]
fn child_uses_owned_directory_after_rename_and_symlink_replacement() -> std::io::Result<()> {
    let scratch = tempfile::tempdir()?;
    let root = scratch.path().join("root");
    let held = scratch.path().join("held");
    let outside = scratch.path().join("outside");
    std::fs::create_dir(&root)?;
    std::fs::create_dir(&outside)?;
    std::fs::write(root.join("marker"), "pinned")?;
    std::fs::write(outside.join("marker"), "replacement")?;
    let directory = File::open(&root)?;
    let parent_cwd = std::env::current_dir()?;
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "cat marker"]);
    current_dir(&mut command, &directory)?;
    drop(directory);
    std::fs::rename(&root, &held)?;
    std::os::unix::fs::symlink(&outside, &root)?;
    let output = command.output()?;
    assert!(output.status.success());
    assert_eq!(output.stdout, b"pinned");
    // A reusable command retains the same owned fd for every spawn.
    assert_eq!(command.output()?.stdout, b"pinned");
    assert_eq!(std::env::current_dir()?, parent_cwd);
    Ok(())
}

#[test]
fn spawning_with_a_file_as_cwd_fails_without_changing_parent_directory() -> std::io::Result<()> {
    let file = tempfile::tempfile()?;
    let parent_cwd = std::env::current_dir()?;
    let mut command = Command::new("/bin/sh");
    command.args(["-c", "exit 0"]);
    current_dir(&mut command, &file)?;
    assert_eq!(
        command.spawn().err().map(|error| error.kind()),
        Some(std::io::ErrorKind::NotADirectory)
    );
    assert_eq!(std::env::current_dir()?, parent_cwd);
    Ok(())
}

#[test]
fn directory_callback_preserves_the_os_error_without_changing_parent_directory()
-> std::io::Result<()> {
    let file = tempfile::tempfile()?;
    let parent_cwd = std::env::current_dir()?;
    let mut callback = directory_callback(file.into());
    assert_eq!(
        callback().err().map(|error| error.kind()),
        Some(std::io::ErrorKind::NotADirectory)
    );
    assert_eq!(std::env::current_dir()?, parent_cwd);
    Ok(())
}
