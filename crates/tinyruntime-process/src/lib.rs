//! Child working-directory binding by an owned directory descriptor.
//!
//! Unix descriptor namespaces are not interchangeable: Darwin's `/dev/fd`
//! cannot stand in for Linux's `/proc/self/fd` directory traversal. Bind the
//! child directly with `fchdir` before exec, without changing the parent cwd.
//!
//! `current_dir` is the Unix entry point. This crate holds only descriptor
//! binding; worker protocols, process pools, and bus routing belong in the
//! runtime crate so this unsafe boundary stays small and independently tested.
//!
//! ```
//! # fn main() -> std::io::Result<()> {
//! # #[cfg(unix)] {
//! use std::fs::File;
//! use std::process::Command;
//!
//! let directory = File::open(std::env::current_dir()?)?;
//! let mut command = Command::new("/bin/sh");
//! command.args(["-c", "pwd"]);
//! tinyruntime_process::current_dir(&mut command, &directory)?;
//! assert!(command.output()?.status.success());
//! # }
//! # Ok(())
//! # }
//! ```

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
    // Keep the clone above stdio: spawn may replace descriptors 0, 1 and 2.
    let directory = rustix::io::fcntl_dupfd_cloexec(directory, 3)?;
    // SAFETY: The closure owns its descriptor, so it cannot be closed/reused
    // before spawn. After fork it performs only the async-signal-safe fchdir
    // syscall and constructs an errno-backed io::Error (no allocation, locks,
    // environment access, or destructors). It never changes the parent cwd.
    unsafe {
        command.pre_exec(directory_callback(directory));
    }
    Ok(())
}

// Keep the exact post-fork operation testable without forking: LLVM counters
// updated in a child that execs are not written into the parent's profile.
#[cfg(unix)]
fn directory_callback(directory: std::os::fd::OwnedFd) -> impl FnMut() -> std::io::Result<()> {
    move || rustix::process::fchdir(&directory).map_err(Into::into)
}

#[cfg(all(test, unix))]
#[path = "lib_tests.rs"]
mod tests;
