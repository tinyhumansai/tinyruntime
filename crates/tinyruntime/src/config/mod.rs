//! The module configuration a host supplies at load time.
//!
//! One decision lives here: which languages this router routes, and to which bus
//! names. It is configuration rather than a compiled-in table because a build
//! that hard-coded its providers would need recompiling to add a language — and
//! the entire point of the provider split is that adding a language is loading
//! another module.
//!
//! A host that supplies nothing gets the first-party providers, which is what
//! almost every host wants.

use serde::{Deserialize, Serialize};

use tinyruntime_bus::{Language, names};

/// What the host told this module at load time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModuleConfig {
    /// The languages to route, and where.
    pub providers: Vec<ProviderRoute>,
    /// Where worker harnesses are written, or empty for a directory under the
    /// platform cache.
    pub harness_dir: String,
}

/// One language and the bus name its provider claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRoute {
    /// The routing key, e.g. `nodejs`.
    pub language: Language,
    /// The well-known bus name the provider module claims.
    pub bus_name: String,
}

impl ProviderRoute {
    /// Route `language` to the module claiming `bus_name`.
    #[must_use]
    pub fn new(language: Language, bus_name: impl Into<String>) -> Self {
        Self {
            language,
            bus_name: bus_name.into(),
        }
    }
}

impl Default for ModuleConfig {
    /// The first-party providers, and the platform cache for harnesses.
    fn default() -> Self {
        Self {
            providers: vec![
                ProviderRoute::new(Language::nodejs(), names::providers::NODEJS),
                ProviderRoute::new(Language::python(), names::providers::PYTHON),
            ],
            harness_dir: String::new(),
        }
    }
}

impl ModuleConfig {
    /// Where worker harnesses should be written.
    ///
    /// A harness is a small script the router writes and then launches, so it
    /// goes in a cache directory rather than anywhere a host would look for its
    /// own data.
    #[must_use]
    pub fn harness_root(&self) -> std::path::PathBuf {
        let configured = self.harness_dir.trim();
        if !configured.is_empty() {
            return std::path::PathBuf::from(configured);
        }
        dirs::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("tinyruntime")
            .join("harnesses")
    }
}

#[cfg(test)]
mod test;
