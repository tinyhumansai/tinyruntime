//! The environment a worker runs in, and where its harness script lives.
//!
//! A worker's environment is built from an allow-list rather than inherited.
//! The host process this module is loaded into holds API keys, tokens, and
//! whatever else its own configuration put there, and a worker runs code that
//! should not be able to read any of it. What a worker needs is a `PATH` that
//! finds its own toolchain first, a home directory, and enough locale and
//! Windows plumbing to start a process at all.

use std::path::{Path, PathBuf};

use tinyruntime_bus::WorkerHarness;

use crate::error::{Error, Result};

/// The variables a worker may inherit from this process.
///
/// Everything not named here is dropped. The Windows entries look like clutter
/// but are load-bearing: after clearing the environment, a Windows child cannot
/// locate its own system libraries or resolve a command without them.
const INHERITED: &[&str] = &[
    "HOME",
    "TERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "USER",
    "SHELL",
    "TMPDIR",
    "SystemRoot",
    "WINDIR",
    "COMSPEC",
    "PATHEXT",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "ProgramW6432",
];

/// Build a worker's environment, with `bin_dir` first on `PATH`.
///
/// Putting the toolchain's own directory first is what makes a job that shells
/// out reach the same interpreter that is running it, rather than whatever the
/// host happens to have installed.
#[must_use]
pub fn build(bin_dir: &Path, extra: &[(String, String)]) -> Vec<(String, String)> {
    let separator = if cfg!(windows) { ";" } else { ":" };
    let inherited_path = std::env::var("PATH").unwrap_or_default();
    let path = if inherited_path.is_empty() {
        bin_dir.to_string_lossy().into_owned()
    } else {
        format!("{}{separator}{inherited_path}", bin_dir.display())
    };

    let mut env = vec![("PATH".to_string(), path)];
    for name in INHERITED {
        if let Ok(value) = std::env::var(name) {
            env.push(((*name).to_string(), value));
        }
    }
    env.extend(extra.iter().cloned());
    env
}

/// Write `harness` into `root` and return the path it was written to.
///
/// Rewritten on each pool build rather than cached across them: a pool is built
/// once per launch fingerprint, and a provider that ships a new harness after an
/// upgrade must not be shadowed by the previous one still sitting on disk.
///
/// # Errors
///
/// Returns [`Error::Storage`] when the directory or the file cannot be written.
pub async fn materialise(root: &Path, harness: &WorkerHarness) -> Result<PathBuf> {
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| Error::Storage(error.to_string()))?;
    let path = root.join(&harness.filename);
    tokio::fs::write(&path, &harness.source)
        .await
        .map_err(|error| Error::Storage(error.to_string()))?;
    tracing::debug!(
        bytes = harness.source.len(),
        "[tinyruntime::pool] wrote the worker harness"
    );
    Ok(path)
}

#[cfg(test)]
#[path = "env_test.rs"]
mod test;
