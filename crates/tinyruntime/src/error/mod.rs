//! The crate-wide error type and result alias.
//!
//! One enum for the whole crate, with a variant per thing that can actually go
//! wrong rather than a string that callers would have to match on. The
//! distinctions here are the ones a caller acts on: a language nobody registered
//! is a configuration problem, a saturated pool is a retry-later, and a
//! post-dispatch worker failure is terminal because the job may already have had
//! its side effects.
//!
//! Messages are lowercase and carry no credential, payload, or absolute path —
//! they are rendered into a host's UI and pasted into bug reports.

use tinyruntime_bus::Language;

/// Everything this crate can fail with.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No provider is registered for the requested language.
    #[error("no runtime provider is registered for `{0}`")]
    UnknownLanguage(Language),

    /// A provider is registered but is not currently serving.
    #[error("the `{language}` runtime provider is not available: {reason}")]
    ProviderUnavailable {
        /// The language whose provider did not answer.
        language: Language,
        /// Why it did not answer, sanitised for display.
        reason: String,
    },

    /// A provider answered, but against an incompatible contract version.
    #[error("the `{language}` runtime provider speaks contract {major}.{minor}, which this build cannot bind to")]
    ProviderContract {
        /// The language whose provider disagreed.
        language: Language,
        /// The major version the provider reported.
        major: u32,
        /// The minor version the provider reported.
        minor: u32,
    },

    /// The host has this language turned off.
    #[error("the `{0}` runtime is disabled")]
    LanguageDisabled(Language),

    /// A request named no language at all.
    #[error("the request named no language")]
    LanguageMissing,

    /// Nothing usable is installed and the request forbade installing.
    #[error("no `{0}` runtime is provisioned and this request did not allow installing one")]
    NotProvisioned(Language),

    /// Fetching an archive or a release index failed.
    #[error("downloading the `{language}` toolchain failed: {reason}")]
    Download {
        /// The language whose toolchain was being fetched.
        language: Language,
        /// What went wrong, sanitised for display.
        reason: String,
    },

    /// A downloaded archive did not hash to the digest the channel published.
    ///
    /// Separate from [`Error::Download`] on purpose: a transfer that failed is a
    /// retry, and a transfer that succeeded and produced the wrong bytes is not.
    #[error("the downloaded `{language}` archive did not match its published digest")]
    DigestMismatch {
        /// The language whose toolchain was being installed.
        language: Language,
    },

    /// Unpacking an archive or promoting an install directory failed.
    #[error("installing the `{language}` toolchain failed: {reason}")]
    Install {
        /// The language whose toolchain was being installed.
        language: Language,
        /// What went wrong, sanitised for display.
        reason: String,
    },

    /// An install finished but the provider found no toolchain in it.
    #[error("the `{0}` toolchain installed but no interpreter was found in it")]
    EmptyInstall(Language),

    /// The pool refused a job because it was already at capacity.
    ///
    /// A caller should retry later or surface a busy state. It must not fall
    /// back to spawning its own interpreter: that reintroduces exactly the
    /// resident memory the pool exists to cap.
    #[error("the `{0}` runtime pool is at capacity")]
    PoolSaturated(Language),

    /// A job failed before it reached a worker, so it never ran.
    ///
    /// Safe to retry: no side effect happened.
    #[error("the `{language}` job could not be dispatched: {reason}")]
    PreDispatch {
        /// The language whose worker was being used.
        language: Language,
        /// What went wrong, sanitised for display.
        reason: String,
    },

    /// A job failed after it reached a worker, so it may have run.
    ///
    /// Terminal: re-running it could duplicate whatever it already did.
    #[error("the `{language}` job failed after dispatch: {reason}")]
    PostDispatch {
        /// The language whose worker was being used.
        language: Language,
        /// What went wrong, sanitised for display.
        reason: String,
    },

    /// A path the module needed could not be read or written.
    #[error("a runtime cache path could not be used: {0}")]
    Storage(String),
}

impl Error {
    /// Whether retrying the same request could plausibly succeed.
    ///
    /// Drives whether a host backs off and tries again or reports the failure to
    /// whoever asked. A digest mismatch and a post-dispatch worker failure are
    /// the two that look transient and are not.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::PoolSaturated(_)
                | Self::PreDispatch { .. }
                | Self::Download { .. }
                | Self::ProviderUnavailable { .. }
        )
    }
}

/// The crate's result alias.
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod test;
