//! An exclusive lock around one install directory.
//!
//! Scoped to the *directory being installed*, not to the cache root: two
//! processes installing two different toolchains into one cache should proceed
//! in parallel, and only a genuine collision should serialise.
//!
//! The lock is a file next to the install directory rather than the directory
//! itself, because the directory is about to be renamed over and a lock on
//! something that is replaced mid-hold is not a lock.

use std::fs::{File, OpenOptions};
use std::path::Path;

use tinyruntime_bus::Language;

use crate::error::{Error, Result};

/// An exclusive install lock, released when dropped.
///
/// The lock lives in the file handle: `fs2` unlocks on close, so holding this
/// value is holding the lock and dropping it releases it. That is why it carries
/// the handle it never reads.
#[derive(Debug)]
pub struct InstallLock {
    _handle: File,
}

impl InstallLock {
    /// Take the exclusive lock guarding `install_dir`, waiting for any other
    /// holder to finish.
    ///
    /// Waiting rather than failing is deliberate: the other holder is almost
    /// always installing the very toolchain this caller wants, so the wait ends
    /// with the work already done.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Install`] when the lock file cannot be created or locked.
    pub async fn acquire(install_dir: &Path, language: &Language) -> Result<Self> {
        let lock_path = install_dir.with_extension("lock");
        if let Some(parent) = lock_path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|error| Error::Storage(error.to_string()))?;
        }

        let owned = language.clone();
        let handle = crate::blocking::run(language, move || -> Result<File> {
            use fs2::FileExt;

            let handle = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(&lock_path)
                .map_err(|error| Error::Install {
                    language: owned.clone(),
                    reason: format!("the install lock could not be opened: {error}"),
                })?;
            handle.lock_exclusive().map_err(|error| Error::Install {
                language: owned.clone(),
                reason: format!("the install lock could not be taken: {error}"),
            })?;
            Ok(handle)
        })
        .await?;

        Ok(Self { _handle: handle })
    }
}
