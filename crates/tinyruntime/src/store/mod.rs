//! Where managed toolchains live on disk, and how one gets there safely.
//!
//! Three problems, all of them the same for every language, all of them easy to
//! get subtly wrong once per language if this module did not exist.
//!
//! **Where.** The default cache root is the user's platform cache directory, not
//! anything relative to a workspace. That is a security choice: a workspace-local
//! default would let a repository vendor a directory shaped like an installed
//! toolchain and have the reuse path pick it up as trusted. A host that sets an
//! explicit cache directory owns that decision.
//!
//! **Who wins.** Two processes sharing a cache root can decide to install the
//! same toolchain at the same moment. An exclusive file lock around the install
//! directory makes the second one wait and then find the first one's work
//! already there, rather than unpacking over it.
//!
//! **How it lands.** A toolchain is promoted with one rename, after it is fully
//! unpacked in a staging directory. A reader either sees the old install or the
//! new one, never a half-populated tree — and if the rename fails, the previous
//! install is put back.

use std::fs;
use std::path::{Path, PathBuf};

use tinyruntime_bus::Language;

use crate::error::{Error, Result};

mod lock;

pub use lock::InstallLock;

/// The directory managed toolchains for `language` are installed under.
///
/// `configured` is a host's explicit choice and is honoured verbatim. Otherwise
/// the platform cache directory is used, and only if the platform has none does
/// this fall back to a hidden directory in the current directory — logged,
/// because that is the one arrangement a repository could try to poison.
#[must_use]
pub fn cache_root(configured: Option<&str>, language: &Language) -> PathBuf {
    if let Some(configured) = configured {
        return PathBuf::from(configured);
    }
    if let Some(platform_cache) = dirs::cache_dir() {
        return platform_cache.join("tinyruntime").join(language.as_str());
    }
    tracing::warn!(
        language = language.as_str(),
        "[tinyruntime::store] no platform cache directory; falling back to a local one (set an explicit cache directory instead)"
    );
    PathBuf::from(".tinyruntime").join(language.as_str())
}

/// Create `root` if it is not already there.
///
/// # Errors
///
/// Returns [`Error::Storage`] when the directory cannot be created.
pub async fn ensure_root(root: &Path) -> Result<()> {
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| Error::Storage(error.to_string()))
}

/// A private staging directory under `root` for one in-progress install.
///
/// Named by process and by a fresh identifier so two installs in one process, or
/// two processes on one cache root, never stage into the same place. A crashed
/// run leaves one behind; [`is_staging`] is how the reuse path knows to skip it.
#[must_use]
pub fn staging_dir(root: &Path) -> PathBuf {
    root.join(format!(
        "{STAGING_PREFIX}{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

/// The prefix every staging directory carries.
const STAGING_PREFIX: &str = ".stage-";

/// Whether `path` is an install-in-progress rather than a finished toolchain.
#[must_use]
pub fn is_staging(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(STAGING_PREFIX))
}

/// Whether `candidate` is a directory that genuinely lives inside `root`.
///
/// Resolves symlinks before comparing, because the whole point is to reject a
/// link planted under the cache root that points somewhere else. A directory the
/// reuse path trusts must be one the cache root actually contains.
#[must_use]
pub fn is_inside(root: &Path, candidate: &Path) -> bool {
    let (Ok(root), Ok(candidate)) = (root.canonicalize(), candidate.canonicalize()) else {
        return false;
    };
    candidate.starts_with(&root)
}

/// Promote a fully unpacked `staged` directory to `destination` with one rename.
///
/// Any existing install is moved aside first and removed only once the promotion
/// has succeeded, so a failure at the rename leaves the previous toolchain in
/// place rather than nothing at all.
///
/// # Errors
///
/// Returns [`Error::Install`] when the destination's parent cannot be created or
/// either rename fails.
pub async fn promote(staged: &Path, destination: &Path, language: &Language) -> Result<()> {
    let staged = staged.to_path_buf();
    let destination = destination.to_path_buf();
    let reported = language.clone();
    let language = language.clone();

    let joined =
        tokio::task::spawn_blocking(move || promote_blocking(&staged, &destination, &language))
            .await;

    match joined {
        Ok(result) => result,
        Err(error) => Err(Error::Install {
            language: reported,
            reason: format!("the install task did not finish: {error}"),
        }),
    }
}

/// The synchronous half of [`promote`].
fn promote_blocking(staged: &Path, destination: &Path, language: &Language) -> Result<()> {
    let install_error = |reason: String| Error::Install {
        language: language.clone(),
        reason,
    };

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            install_error(format!("the cache root could not be created: {error}"))
        })?;
    }

    let displaced = if destination.exists() {
        let aside = destination.with_extension(format!("replaced-{}", std::process::id()));
        let _ = fs::remove_dir_all(&aside);
        fs::rename(destination, &aside).map_err(|error| {
            install_error(format!(
                "the existing install could not be moved aside: {error}"
            ))
        })?;
        Some(aside)
    } else {
        None
    };

    if let Err(error) = fs::rename(staged, destination) {
        if let Some(aside) = displaced.as_ref() {
            if let Err(restore) = fs::rename(aside, destination) {
                return Err(install_error(format!(
                    "the toolchain could not be installed ({error}) and the previous one could not be restored ({restore})"
                )));
            }
            tracing::info!(
                "[tinyruntime::store] restored the previous install after a failed promotion"
            );
        }
        return Err(install_error(format!(
            "the unpacked toolchain could not be moved into place: {error}"
        )));
    }

    if let Some(aside) = displaced {
        let _ = fs::remove_dir_all(aside);
    }
    Ok(())
}

/// Remove a directory and everything under it, best effort.
///
/// Used for staging cleanup, where a failure is worth a log line and nothing
/// more: the install already succeeded, and the leftover is skipped on reuse.
pub async fn discard(path: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::debug!(
                "[tinyruntime::store] a staging directory could not be removed: {error}"
            );
        }
    }
}

#[cfg(test)]
mod test;
