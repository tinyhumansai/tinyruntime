//! Installing a managed toolchain, once, safely, and never twice at a time.
//!
//! The sequence is: ask the provider what to install, take the lock for that
//! install directory, check again whether it is already there, download, verify,
//! unpack into a staging directory, promote with one rename, and clean up.
//!
//! The second check — after the lock, before the download — is the part that is
//! easy to leave out and expensive to omit. Two processes that decide to install
//! the same toolchain at the same moment both pass the first check; the lock
//! makes the second one wait, and the re-check makes it discover the first one's
//! work instead of downloading several hundred megabytes to overwrite it.

use reqwest::Client;

use tinyruntime_bus::{Language, ResolvedRuntime, RuntimeSettings};

use crate::error::{Error, Result};
use crate::provider::Provider;
use crate::{archive, download, store};

/// Provision a managed toolchain and return it.
///
/// # Errors
///
/// Returns the download, digest, install, and empty-install variants of
/// [`Error`] as each stage can fail.
pub(super) async fn run(
    client: &Client,
    language: &Language,
    settings: &RuntimeSettings,
    provider: &dyn Provider,
) -> Result<ResolvedRuntime> {
    let distribution = provider.select_distribution(settings).await?;
    let root = store::cache_root(settings.cache_dir(), language);
    store::ensure_root(&root).await?;

    let install_dir = root.join(&distribution.install_dir_name);
    let _lock = store::InstallLock::acquire(&install_dir, language).await?;

    let install_path = install_dir.to_string_lossy().into_owned();
    if let Some(layout) = provider.layout(&install_path, settings).await? {
        tracing::info!(
            language = language.as_str(),
            version = %layout.version,
            "[tinyruntime::resolve] another install finished this toolchain while we waited"
        );
        return Ok(super::managed(language, &install_dir, layout));
    }

    tracing::info!(
        language = language.as_str(),
        version = %distribution.version,
        "[tinyruntime::resolve] installing a managed toolchain"
    );

    let archive_path = root.join(&distribution.archive_name);
    download::fetch(client, &distribution, &archive_path, language).await?;

    let staging = store::staging_dir(&root);
    store::discard(&staging).await;

    let unpacked = match archive::extract(&archive_path, &staging, distribution.format, language).await
    {
        Ok(unpacked) => unpacked,
        Err(error) => {
            store::discard(&staging).await;
            remove_archive(&archive_path).await;
            return Err(error);
        }
    };

    let promoted = store::promote(&unpacked, &install_dir, language).await;
    store::discard(&staging).await;
    remove_archive(&archive_path).await;
    promoted?;

    provider
        .layout(&install_path, settings)
        .await?
        .map(|layout| super::managed(language, &install_dir, layout))
        .ok_or_else(|| Error::EmptyInstall(language.clone()))
}

/// Drop the archive once it has been unpacked.
///
/// Kept until this point rather than streamed straight into the extractor so a
/// failed unpack can be diagnosed against the bytes that caused it, and removed
/// afterwards because a verified archive is several hundred megabytes of no
/// further use.
async fn remove_archive(path: &std::path::Path) {
    if let Err(error) = tokio::fs::remove_file(path).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::debug!("[tinyruntime::resolve] the archive could not be removed: {error}");
        }
    }
}
