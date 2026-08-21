//! Finding a managed toolchain that is already installed.
//!
//! This is the step that makes a warm restart cheap. Without it, a host that
//! installed Node last week would ask its provider for a release index and then
//! discover it already had the answer — which costs a network round trip on
//! every process start, and fails outright when the machine is offline.
//!
//! The scan asks the provider about each candidate directory rather than
//! deciding for itself, because "is this a usable toolchain for these settings"
//! is a language question. It also refuses anything that is not genuinely inside
//! the cache root: a symlink planted there would otherwise let a directory from
//! anywhere on the filesystem be reused as a trusted install.

use std::path::Path;

use tinyruntime_bus::{Language, ResolvedRuntime, RuntimeSettings};

use crate::provider::Provider;
use crate::store;

/// Look through `root` for an installed toolchain these settings accept.
///
/// Never fails: a cache that cannot be read is a cache with nothing in it, and
/// turning that into an error would break a host whose cache directory does not
/// exist yet — which is every host, once.
pub(super) async fn scan(
    root: &Path,
    language: &Language,
    settings: &RuntimeSettings,
    provider: &dyn Provider,
) -> Option<ResolvedRuntime> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return None;
    };

    let mut candidates: Vec<std::path::PathBuf> = entries
        .flatten()
        .filter(|entry| {
            // `file_type` reports the link itself, unlike `is_dir`, which follows
            // it. Skipping links here is what stops a planted one from being
            // reused as a trusted install.
            entry.file_type().is_ok_and(|kind| kind.is_dir())
        })
        .map(|entry| entry.path())
        .filter(|path| !store::is_staging(path))
        .filter(|path| store::is_inside(root, path))
        .collect();

    // Descending, so a cache holding several versions offers the newest-looking
    // one first and the scan is deterministic rather than directory-order.
    candidates.sort();
    candidates.reverse();

    for candidate in candidates {
        let path = candidate.to_string_lossy();
        match provider.layout(&path, settings).await {
            Ok(Some(layout)) => {
                tracing::info!(
                    language = language.as_str(),
                    version = %layout.version,
                    "[tinyruntime::resolve] reusing a managed toolchain already installed"
                );
                return Some(super::managed(language, &candidate, layout));
            }
            Ok(None) => {}
            Err(error) => {
                tracing::debug!(
                    language = language.as_str(),
                    "[tinyruntime::resolve] a cached directory could not be inspected: {error}"
                );
            }
        }
    }
    None
}
