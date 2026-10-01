//! The worker script and how it is put on disk.
//!
//! The script is embedded so the interpreter always runs the revision that
//! matches this crate's [`PROTOCOL_VERSION`](crate::PROTOCOL_VERSION). It is
//! rewritten on every launch preparation, never trusted from a previous run.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// The worker script, embedded.
///
/// It reads its backend list and tuning from `OPENHUMAN_RPS_*` environment
/// variables, which the host puts in [`ServerLaunch::env`](crate::ServerLaunch).
pub const SERVER_SCRIPT: &str = include_str!("server.py");

/// The file name the script is written under.
pub const SCRIPT_FILE_NAME: &str = "runtime_python_server.py";

/// Write [`SERVER_SCRIPT`] into `root`, creating it if needed, and return its
/// path.
///
/// # Errors
///
/// Returns [`Error::CreateDir`] or [`Error::WriteScript`] when the filesystem
/// refuses.
pub async fn write_script(root: &Path) -> Result<PathBuf> {
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|source| Error::CreateDir {
            path: root.display().to_string(),
            source,
        })?;
    let path = root.join(SCRIPT_FILE_NAME);
    tokio::fs::write(&path, SERVER_SCRIPT)
        .await
        .map_err(|source| Error::WriteScript {
            path: path.display().to_string(),
            source,
        })?;
    Ok(path)
}

#[cfg(test)]
#[path = "script_tests.rs"]
mod tests;
