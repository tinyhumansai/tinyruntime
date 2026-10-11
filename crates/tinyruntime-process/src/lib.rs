//! Child working-directory binding by an owned directory descriptor.
//!
//! Unix descriptor namespaces are not interchangeable: Darwin's `/dev/fd`
//! cannot stand in for Linux's `/proc/self/fd` directory traversal. Bind the
//! child directly with `fchdir` before exec, without changing the parent cwd.

/// Pin a Unix command's working directory to an owned clone of a directory fd.
///
/// The descriptor remains alive in the command until spawning completes. No
/// pathname is reopened, even if the directory is renamed or replaced.
///
/// # Errors
/// Returns an error if cloning the descriptor fails. A non-directory descriptor
/// causes spawning to fail with the operating system's `fchdir` error.
#[cfg(unix)]
#[allow(unsafe_code, reason = "minimal async-signal-safe pre_exec boundary")]
pub fn current_dir(
    command: &mut std::process::Command,
    directory: impl std::os::fd::AsFd,
) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    let directory = directory.as_fd().try_clone_to_owned()?;
    // SAFETY: The closure owns its descriptor, so it cannot be closed/reused
    // before spawn. After fork it performs only the async-signal-safe fchdir
    // syscall and constructs an errno-backed io::Error (no allocation, locks,
    // environment access, or destructors). It never changes the parent cwd.
    unsafe {
        command.pre_exec(move || rustix::process::fchdir(&directory).map_err(Into::into));
    }
    Ok(())
}

#[cfg(all(test, unix))]
#[path = "lib_tests.rs"]
mod tests;
