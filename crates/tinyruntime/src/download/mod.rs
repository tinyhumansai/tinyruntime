//! Fetching an archive and proving it is the archive the channel published.
//!
//! The digest check is not a defensive extra. These bytes become an interpreter
//! that this host then runs code with, so an archive that arrives intact but
//! wrong is the worst outcome available — worse than a failed download, which at
//! least fails loudly. Verification therefore has no opt-out, and a mismatch
//! deletes the file rather than leaving it somewhere a later run might reuse.
//!
//! The one concession: a channel that publishes no digest at all still installs,
//! because refusing would make that language unusable rather than safer. It is
//! logged at warning level every time.

use std::path::Path;

use reqwest::Client;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use tinyruntime_bus::{Distribution, Language};

use crate::error::{Error, Result};

/// Stream `distribution` to `target`, hashing as it goes, and verify the digest.
///
/// The archive is hashed chunk by chunk while it is written rather than read
/// back afterwards: a toolchain archive is hundreds of megabytes, and buffering
/// one in memory to check it would cost more than the install it enables.
///
/// # Errors
///
/// Returns [`Error::Download`] when the request or the transfer fails, and
/// [`Error::DigestMismatch`] when the completed bytes hash to something other
/// than the digest the channel published. Both delete the partial file.
pub async fn fetch(
    client: &Client,
    distribution: &Distribution,
    target: &Path,
    language: &Language,
) -> Result<()> {
    tracing::info!(
        archive = %distribution.archive_name,
        "[tinyruntime::download] fetching toolchain archive"
    );

    if let Some(parent) = target.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| Error::Storage(error.to_string()))?;
    }

    let digest = match stream_to_file(client, distribution, target, language).await {
        Ok(digest) => digest,
        Err(error) => {
            remove_partial(target).await;
            return Err(error);
        }
    };

    match distribution.expected_sha256.as_deref() {
        Some(expected) if !expected.eq_ignore_ascii_case(&digest) => {
            remove_partial(target).await;
            tracing::error!(
                archive = %distribution.archive_name,
                "[tinyruntime::download] archive digest did not match the published one"
            );
            Err(Error::DigestMismatch {
                language: language.clone(),
            })
        }
        Some(_) => {
            tracing::info!(
                archive = %distribution.archive_name,
                "[tinyruntime::download] archive verified against its published digest"
            );
            Ok(())
        }
        None => {
            tracing::warn!(
                archive = %distribution.archive_name,
                "[tinyruntime::download] channel published no digest; installing an unverified toolchain"
            );
            Ok(())
        }
    }
}

/// Write the response body to `target`, returning the hex SHA-256 of what was
/// written.
async fn stream_to_file(
    client: &Client,
    distribution: &Distribution,
    target: &Path,
    language: &Language,
) -> Result<String> {
    let mut request = client.get(&distribution.url).header(
        reqwest::header::USER_AGENT,
        concat!("tinyruntime/", env!("CARGO_PKG_VERSION")),
    );
    for (name, value) in &distribution.headers {
        request = request.header(name.as_str(), value.as_str());
    }

    let mut response = request
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| download_error(language, &error))?;

    let mut file = tokio::fs::File::create(target)
        .await
        .map_err(|error| Error::Storage(error.to_string()))?;
    let mut hasher = Sha256::new();

    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| download_error(language, &error))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| Error::Storage(error.to_string()))?;
    }
    file.flush()
        .await
        .map_err(|error| Error::Storage(error.to_string()))?;

    Ok(hex::encode(hasher.finalize()))
}

/// Delete a partial or rejected download, best effort.
///
/// Leaving it behind is the real risk: a later run that finds an archive already
/// on disk should never be able to reuse bytes that failed verification.
async fn remove_partial(target: &Path) {
    if let Err(error) = tokio::fs::remove_file(target).await {
        if error.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!(
                "[tinyruntime::download] a rejected archive could not be removed: {error}"
            );
        }
    }
}

/// Render a transfer failure without putting the URL into a host-visible message.
fn download_error(language: &Language, error: &reqwest::Error) -> Error {
    Error::Download {
        language: language.clone(),
        reason: sanitise(error),
    }
}

/// Describe a request failure by its kind rather than by its `Display`, which
/// embeds the full URL.
fn sanitise(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "the request timed out".to_string()
    } else if error.is_connect() {
        "the connection could not be established".to_string()
    } else if let Some(status) = error.status() {
        format!("the channel answered with status {status}")
    } else if error.is_body() || error.is_decode() {
        "the transfer ended early".to_string()
    } else {
        "the request failed".to_string()
    }
}

#[cfg(test)]
mod test;
