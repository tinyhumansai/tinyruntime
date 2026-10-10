//! What a provider knows about its distribution channel and its install layout.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The archive shapes the router can unpack.
///
/// A provider names the shape; the router owns the unpacking, so a new provider
/// never carries its own decompressor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ArchiveFormat {
    /// A gzip-compressed tarball.
    TarGz,
    /// An xz-compressed tarball.
    TarXz,
    /// A zip archive.
    Zip,
}

impl ArchiveFormat {
    /// The conventional file extension for this format, without a leading dot.
    ///
    /// # Examples
    ///
    /// ```
    /// # use tinyruntime_bus::ArchiveFormat;
    /// assert_eq!(ArchiveFormat::TarXz.extension(), "tar.xz");
    /// ```
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::TarGz => "tar.gz",
            Self::TarXz => "tar.xz",
            Self::Zip => "zip",
        }
    }
}

/// One downloadable toolchain, fully addressed for this host.
///
/// The provider resolves this — it is the part that needs to know a release
/// index, a filename convention, and a host triple. Everything after it (fetch,
/// verify, unpack, promote) is language-agnostic and belongs to the router.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Distribution {
    /// Human-readable version this archive installs, e.g. `22.11.0`.
    pub version: String,
    /// Archive filename, used for the staging path and for logging.
    pub archive_name: String,
    /// Absolute URL to fetch.
    pub url: String,
    /// Lowercase hex SHA-256 the downloaded bytes must hash to.
    ///
    /// `None` means the channel published no digest. The router still installs,
    /// because refusing would make the language unusable on that channel, but it
    /// says so loudly — an unverified toolchain runs code on this host.
    pub expected_sha256: Option<String>,
    /// How to unpack the archive.
    pub format: ArchiveFormat,
    /// Directory name for the finished install, relative to the cache root.
    ///
    /// Deriving it from the archive rather than from the version is what makes a
    /// warm restart find the same directory without asking the network again.
    pub install_dir_name: String,
    /// Extra request headers the channel requires, such as an API accept header.
    #[serde(default)]
    pub headers: Vec<(String, String)>,
}

impl Distribution {
    /// Builds a distribution descriptor with no extra headers.
    #[must_use]
    pub fn new(
        version: impl Into<String>,
        archive_name: impl Into<String>,
        url: impl Into<String>,
        format: ArchiveFormat,
    ) -> Self {
        let archive_name = archive_name.into();
        let install_dir_name = archive_name
            .strip_suffix(&format!(".{}", format.extension()))
            .unwrap_or(&archive_name)
            .to_owned();
        Self {
            version: version.into(),
            archive_name,
            url: url.into(),
            expected_sha256: None,
            format,
            install_dir_name,
            headers: Vec::new(),
        }
    }

    /// Sets the digest the downloaded bytes must match.
    #[must_use]
    pub fn with_sha256(mut self, digest: impl Into<String>) -> Self {
        self.expected_sha256 = Some(digest.into());
        self
    }

    /// Overrides the directory name the finished install is promoted to.
    #[must_use]
    pub fn with_install_dir_name(mut self, name: impl Into<String>) -> Self {
        self.install_dir_name = name.into();
        self
    }

    /// Adds a request header the distribution channel requires.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// Where the executables of an installed toolchain are.
///
/// A provider reports this for an extracted directory, because only it knows
/// that Node.js puts `node` under `bin/` on Unix and at the root on Windows, or
/// that a standalone `CPython` may be reachable as any of several names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RuntimeLayout {
    /// Version the installed toolchain reports.
    pub version: String,
    /// Directory to prepend to a child's `PATH` so the toolchain's own tools win.
    pub bin_dir: String,
    /// Absolute path per logical executable name, e.g. `node`, `npm`, `python`.
    ///
    /// Logical names are the contract; a host asks for `node` and gets whatever
    /// the provider decided that means on this platform.
    pub executables: BTreeMap<String, String>,
}

impl RuntimeLayout {
    /// Builds a layout rooted at `bin_dir` with no executables recorded yet.
    #[must_use]
    pub fn new(version: impl Into<String>, bin_dir: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            bin_dir: bin_dir.into(),
            executables: BTreeMap::new(),
        }
    }

    /// Records `path` as the toolchain's `name` executable.
    #[must_use]
    pub fn with_executable(mut self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.executables.insert(name.into(), path.into());
        self
    }

    /// The absolute path recorded for `name`, if the toolchain ships it.
    #[must_use]
    pub fn executable(&self, name: &str) -> Option<&str> {
        self.executables.get(name).map(String::as_str)
    }
}

/// What a provider is and what it targets by default.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProviderDescriptor {
    /// The language this provider serves, matching its routing key.
    pub language: crate::Language,
    /// Name for an operator-facing listing, e.g. `Node.js`.
    pub display_name: String,
    /// The version this provider targets when a host expresses no preference.
    pub default_version: String,
    /// The contract version this provider was built against.
    pub contract_version: (u32, u32),
    /// The logical executable names this provider's layouts can carry.
    pub executables: Vec<String>,
    /// Optional language capabilities implemented by this provider.
    ///
    /// Omitted from the wire when empty, preserving the descriptor shape used
    /// by providers that predate optional capabilities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<ProviderCapability>,
}

impl ProviderDescriptor {
    /// Builds a descriptor reporting the compiled-in [`crate::CONTRACT_VERSION`].
    #[must_use]
    pub fn new(
        language: crate::Language,
        display_name: impl Into<String>,
        default_version: impl Into<String>,
    ) -> Self {
        Self {
            language,
            display_name: display_name.into(),
            default_version: default_version.into(),
            contract_version: crate::CONTRACT_VERSION,
            executables: Vec::new(),
            capabilities: Vec::new(),
        }
    }

    /// Declares a logical executable name this provider reports in its layouts.
    #[must_use]
    pub fn with_executable(mut self, name: impl Into<String>) -> Self {
        self.executables.push(name.into());
        self
    }

    /// Declares an optional provider operation this module implements.
    #[must_use]
    pub fn with_capability(mut self, capability: ProviderCapability) -> Self {
        if !self.capabilities.contains(&capability) {
            self.capabilities.push(capability);
        }
        self
    }
}

/// An optional operation a provider can describe in addition to the base
/// provider interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProviderCapability {
    /// Describe how to create a language environment and install named packages.
    PrepareEnvironment,
    /// A newer provider capability this router does not implement.
    ///
    /// Unknown values are accepted and ignored during descriptor decoding so a
    /// provider can add an unrelated capability without breaking older routers.
    #[serde(other)]
    Unknown,
}

/// A host's language-scoped request to the router's provider preparation bridge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrepareEnvironmentRequest {
    /// Language whose configured provider should describe the recipe.
    pub language: crate::Language,
    /// Generic provider inputs selected by the recipe owner and host.
    pub preparation: EnvironmentPreparationRequest,
}

impl PrepareEnvironmentRequest {
    /// Builds a bridge request for one language.
    #[must_use]
    pub fn new(language: crate::Language, preparation: EnvironmentPreparationRequest) -> Self {
        Self {
            language,
            preparation,
        }
    }
}

/// Host-selected inputs for a provider's declarative environment recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPreparationRequest {
    /// Absolute runtime executable reported by this provider's layout.
    pub runtime_executable: String,
    /// Host-approved absolute directory for the prepared environment.
    pub environment_root: String,
    /// Whether the provider should describe an installer self-upgrade step.
    pub upgrade_installer: bool,
    /// Ordered package groups selected by the recipe owner.
    pub package_groups: Vec<PackageGroup>,
}

impl EnvironmentPreparationRequest {
    /// Builds a request from the resolved runtime and host-approved directory.
    #[must_use]
    pub fn new(runtime_executable: impl Into<String>, environment_root: impl Into<String>) -> Self {
        Self {
            runtime_executable: runtime_executable.into(),
            environment_root: environment_root.into(),
            upgrade_installer: false,
            package_groups: Vec::new(),
        }
    }

    /// Requests an installer self-upgrade before package groups.
    #[must_use]
    pub fn with_installer_upgrade(mut self) -> Self {
        self.upgrade_installer = true;
        self
    }

    /// Adds a package group in provider installation order.
    #[must_use]
    pub fn with_package_group(mut self, group: PackageGroup) -> Self {
        self.package_groups.push(group);
        self
    }
}

/// An ordered group of named requirements and an optional source hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageGroup {
    /// Names selected by the recipe owner, passed as individual arguments.
    pub packages: Vec<String>,
    /// Optional source hint interpreted by the provider's installer.
    pub source: Option<String>,
}

impl PackageGroup {
    /// Builds a group with no source override.
    #[must_use]
    pub fn new(packages: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            packages: packages.into_iter().map(Into::into).collect(),
            source: None,
        }
    }

    /// Sets the source hint for these packages.
    #[must_use]
    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

/// Provider-authored native commands and executable path for one environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvironmentPreparationPlan {
    /// Absolute executable inside the prepared environment.
    pub executable: String,
    /// Ordered native commands that create and populate the environment.
    pub steps: Vec<crate::worker::WorkerCommand>,
}

/// Ask a provider where the executables are inside an extracted install.
///
/// The settings ride along because the router reuses installs by scanning its
/// cache, and only the provider can say whether the toolchain in a given
/// directory satisfies what the caller asked for. Without them the router would
/// happily reuse a Python 3.11 install for a request that needs 3.12.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LayoutRequest {
    /// Absolute path to a directory that may hold an installed toolchain.
    pub install_dir: String,
    /// What the caller asked for, so an unsuitable install can be declined.
    pub settings: crate::RuntimeSettings,
}

impl LayoutRequest {
    /// Builds a request about `install_dir` under `settings`.
    #[must_use]
    pub fn new(install_dir: impl Into<String>, settings: crate::RuntimeSettings) -> Self {
        Self {
            install_dir: install_dir.into(),
            settings,
        }
    }
}

/// A provider's answer about a toolchain that may or may not be there.
///
/// Both [`crate::names::provider_methods::DETECT_SYSTEM`] and
/// [`crate::names::provider_methods::LAYOUT`] answer with this, because both ask
/// the same question of different places: is there a usable toolchain here, and
/// if so, where are its parts? "No" is an ordinary answer to that, not a fault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct LayoutResponse {
    /// The toolchain found, or `None` when there is none.
    pub layout: Option<RuntimeLayout>,
}

impl LayoutResponse {
    /// A reply carrying a toolchain.
    #[must_use]
    pub fn found(layout: RuntimeLayout) -> Self {
        Self {
            layout: Some(layout),
        }
    }

    /// A reply reporting that there is no usable toolchain here.
    #[must_use]
    pub fn missing() -> Self {
        Self { layout: None }
    }
}
