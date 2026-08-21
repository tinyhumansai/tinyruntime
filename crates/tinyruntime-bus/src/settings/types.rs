//! The per-language knobs a host hands the router with every request.

use serde::{Deserialize, Serialize};

/// How a host wants one language's runtime resolved.
///
/// The router never reads configuration of its own: every request carries the
/// settings it should be served under, so a host that changes a version pin or a
/// cache directory does not need the module to reload. Provider-specific meaning
/// is deliberate — [`RuntimeSettings::version`] is an exact pin for Node.js and a
/// minimum floor for Python, because that is what each distribution channel
/// actually offers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RuntimeSettings {
    /// Whether this language may be used at all. A disabled language fails its
    /// requests with a clear reason instead of silently falling back.
    pub enabled: bool,
    /// Reuse a compatible interpreter already on the host before downloading a
    /// managed one. Turning this off forces the managed toolchain, which is what
    /// a caller wants when it needs an exact version.
    pub prefer_system: bool,
    /// The version the provider should target. An exact pin for channels that
    /// publish one archive per version, a lower bound for channels that publish
    /// a moving set.
    pub version: String,
    /// Exclusive upper version bound, or empty for none. Keeps selection off a
    /// newer pre-release series when the channel publishes those alongside.
    pub maximum_version: String,
    /// Where managed installs live, or empty for the platform cache directory.
    ///
    /// A host that sets this owns the path: the router creates it, installs
    /// under it, and reuses whatever it finds there.
    pub cache_dir: String,
    /// A provider-specific release pin (a distribution channel's tag), or empty
    /// for whatever that channel calls current.
    pub release_tag: String,
    /// An interpreter command to try before the provider's own candidates, or
    /// empty for none.
    pub preferred_command: String,
}

impl RuntimeSettings {
    /// Builds settings that enable a language at `version`, preferring a
    /// compatible host interpreter and the platform cache directory.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinyruntime_bus::RuntimeSettings;
    /// let settings = RuntimeSettings::new("v22.11.0");
    /// assert!(settings.enabled && settings.prefer_system);
    /// assert!(settings.cache_dir.is_empty());
    /// ```
    #[must_use]
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            enabled: true,
            prefer_system: true,
            version: version.into(),
            maximum_version: String::new(),
            cache_dir: String::new(),
            release_tag: String::new(),
            preferred_command: String::new(),
        }
    }

    /// The configured cache directory, or `None` when the platform default
    /// should be used.
    #[must_use]
    pub fn cache_dir(&self) -> Option<&str> {
        non_empty(&self.cache_dir)
    }

    /// The configured release pin, or `None` for the channel's current release.
    #[must_use]
    pub fn release_tag(&self) -> Option<&str> {
        non_empty(&self.release_tag)
    }

    /// The caller's preferred interpreter command, or `None`.
    #[must_use]
    pub fn preferred_command(&self) -> Option<&str> {
        non_empty(&self.preferred_command)
    }

    /// The exclusive upper version bound, or `None` when unbounded.
    #[must_use]
    pub fn maximum_version(&self) -> Option<&str> {
        non_empty(&self.maximum_version)
    }
}

/// Treat a blank field as absent.
///
/// Every optional field here is a `String` rather than an `Option<String>`
/// because these values come from a host's TOML configuration, where "unset" and
/// "set to empty" are the same thing to the person editing the file.
fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}
