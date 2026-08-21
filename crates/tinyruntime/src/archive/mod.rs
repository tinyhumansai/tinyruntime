//! Unpacking a toolchain archive, whatever compression the channel chose.
//!
//! Three formats cover every release channel a provider has needed so far, and a
//! provider names which one it published rather than shipping a decompressor of
//! its own. That is the whole reason this module is here instead of three times
//! over in the provider crates.
//!
//! Every toolchain archive is single-rooted: it expands into exactly one
//! directory named for the release. This module extracts into a caller-supplied
//! staging directory and returns that inner directory, so [`crate::store`] can
//! promote it into the cache with one rename.
//!
//! Extraction is CPU- and IO-bound and the underlying crates are synchronous, so
//! the real work runs on a blocking thread rather than stalling the async
//! runtime the module shares with the bus.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use tinyruntime_bus::ArchiveFormat;

use crate::error::{Error, Result};

mod extract;

/// Extract `archive` into `staging_dir` and return the single directory it
/// produced.
///
/// The caller owns `staging_dir` and should treat it as contaminated if this
/// fails — a half-unpacked tree is not something to retry into.
///
/// # Errors
///
/// Returns [`Error::Install`] when the archive cannot be read, cannot be
/// unpacked, or does not expand into exactly one directory.
pub async fn extract(
    archive: &Path,
    staging_dir: &Path,
    format: ArchiveFormat,
    language: &tinyruntime_bus::Language,
) -> Result<PathBuf> {
    let archive = archive.to_path_buf();
    let staging_dir = staging_dir.to_path_buf();
    let language = language.clone();

    tracing::info!(
        archive = %archive.display(),
        staging_dir = %staging_dir.display(),
        format = format.extension(),
        "[tinyruntime::archive] unpacking toolchain archive"
    );

    let reported = language.clone();
    let joined = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        fs::create_dir_all(&staging_dir).map_err(|error| install_error(&language, &error))?;
        // A format this build does not know is a provider from a newer contract,
        // which the router refuses long before here — but the payload type is
        // `#[non_exhaustive]`, so the arm has to exist and must not panic.
        let unpacked = match format {
            ArchiveFormat::TarGz => extract::tar_gz(&archive, &staging_dir),
            ArchiveFormat::TarXz => extract::tar_xz(&archive, &staging_dir),
            ArchiveFormat::Zip => extract::zip(&archive, &staging_dir),
            _ => {
                return Err(Error::Install {
                    language,
                    reason: "the provider named an archive format this build cannot unpack"
                        .to_string(),
                });
            }
        };
        unpacked.map_err(|error| install_error(&language, &error))?;
        single_root(&staging_dir).map_err(|reason| Error::Install {
            language: language.clone(),
            reason,
        })
    })
    .await;

    match joined {
        Ok(result) => result,
        Err(error) => Err(Error::Install {
            language: reported,
            reason: format!("the unpacking task did not finish: {error}"),
        }),
    }
}

/// Locate the single top-level directory inside `staging_dir`.
///
/// Anything else — several directories, or none — means the archive was not what
/// the provider said it was, and guessing which directory to promote would
/// install something nobody chose.
fn single_root(staging_dir: &Path) -> std::result::Result<PathBuf, String> {
    let entries = fs::read_dir(staging_dir)
        .map_err(|error| format!("the unpacked directory could not be listed: {error}"))?;

    let mut directories: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("an unpacked entry could not be read: {error}"))?;
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            directories.push(entry.path());
        }
    }
    directories.sort();

    match directories.len() {
        1 => directories.pop().ok_or_else(|| {
            "the unpacked directory disappeared while being read".to_string()
        }),
        0 => Err("the archive expanded into no directory at all".to_string()),
        found => Err(format!(
            "the archive expanded into {found} directories, not one"
        )),
    }
}

/// Wrap an IO failure as an install failure without leaking the path into the
/// message a host renders.
fn install_error(language: &tinyruntime_bus::Language, error: &io::Error) -> Error {
    Error::Install {
        language: language.clone(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod test;
