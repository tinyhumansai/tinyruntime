//! Requests and replies for resolving a language runtime.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Language, RuntimeLayout, RuntimeSettings};

/// Where a resolved toolchain came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RuntimeSource {
    /// A compatible toolchain already on the host was reused.
    System,
    /// A managed toolchain the router downloaded and installed.
    Managed,
}

/// Ask the router for a usable toolchain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResolveRequest {
    /// Which language to resolve.
    pub language: Language,
    /// How the caller wants it resolved.
    pub settings: RuntimeSettings,
    /// Whether the router may download and install a managed toolchain.
    ///
    /// `false` turns the call into a readiness probe: it reports what is already
    /// on the host and returns nothing rather than spending a caller's latency
    /// budget on a multi-hundred-megabyte download it did not ask for.
    pub install: bool,
}

impl ResolveRequest {
    /// Builds a request that may install a managed toolchain.
    #[must_use]
    pub fn new(language: Language, settings: RuntimeSettings) -> Self {
        Self {
            language,
            settings,
            install: true,
        }
    }

    /// Builds a non-installing readiness probe.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinyruntime_bus::{Language, ResolveRequest, RuntimeSettings};
    /// let probe = ResolveRequest::probe(Language::nodejs(), RuntimeSettings::new("v22.11.0"));
    /// assert!(!probe.install);
    /// ```
    #[must_use]
    pub fn probe(language: Language, settings: RuntimeSettings) -> Self {
        Self {
            language,
            settings,
            install: false,
        }
    }
}

/// A toolchain the caller can run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResolvedRuntime {
    /// The language this toolchain serves.
    pub language: Language,
    /// The version it reports.
    pub version: String,
    /// Whether it was found on the host or installed by the router.
    pub source: RuntimeSource,
    /// Directory to prepend to a child's `PATH`.
    pub bin_dir: String,
    /// Absolute path per logical executable name.
    pub executables: BTreeMap<String, String>,
    /// The managed install directory, absent for a reused system toolchain.
    pub install_dir: Option<String>,
}

impl ResolvedRuntime {
    /// Builds a resolution from the layout a provider reported.
    #[must_use]
    pub fn from_layout(language: Language, source: RuntimeSource, layout: RuntimeLayout) -> Self {
        Self {
            language,
            version: layout.version,
            source,
            bin_dir: layout.bin_dir,
            executables: layout.executables,
            install_dir: None,
        }
    }

    /// Records the managed directory this toolchain was installed into.
    #[must_use]
    pub fn with_install_dir(mut self, install_dir: impl Into<String>) -> Self {
        self.install_dir = Some(install_dir.into());
        self
    }

    /// The absolute path recorded for `name`, if the toolchain ships it.
    #[must_use]
    pub fn executable(&self, name: &str) -> Option<&str> {
        self.executables.get(name).map(String::as_str)
    }
}

/// The reply to a resolve request.
///
/// A probe that finds nothing is a successful call with an empty answer, not an
/// error: "not installed yet" is a normal state a host renders, and making the
/// caller distinguish it from a genuine failure by matching on error text would
/// be worse than a variant that says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResolveResponse {
    /// The resolved toolchain, or `None` when a probe found nothing installed.
    pub runtime: Option<ResolvedRuntime>,
}

impl ResolveResponse {
    /// A reply carrying a resolved toolchain.
    #[must_use]
    pub fn found(runtime: ResolvedRuntime) -> Self {
        Self {
            runtime: Some(runtime),
        }
    }

    /// A reply reporting that nothing is provisioned yet.
    #[must_use]
    pub fn missing() -> Self {
        Self { runtime: None }
    }
}

/// One language the router can route to, and whether it currently can.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LanguageStatus {
    /// The routing key.
    pub language: Language,
    /// The well-known bus name the provider is expected to claim.
    pub bus_name: String,
    /// Whether the provider answered when the router last asked.
    pub available: bool,
    /// The provider's operator-facing name, when it answered.
    pub display_name: Option<String>,
    /// Why the provider is unavailable, when it is.
    ///
    /// Never carries a path, a URL, or a payload: this is rendered into a UI and
    /// pasted into bug reports.
    pub detail: Option<String>,
}

/// The reply listing every language this router knows how to route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LanguagesResponse {
    /// One entry per registered provider, in registration order.
    pub languages: Vec<LanguageStatus>,
}
