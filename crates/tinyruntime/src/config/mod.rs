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
///
/// Arrives as JSON in the loader's configuration slot. A host that configures
/// nothing supplies either the empty object or `null`, so both must decode to
/// the same thing: the first-party providers and the platform cache.
///
/// `#[serde(default)]` on [`Wire`] covers the empty object, because it fills in
/// absent *fields*. It does not cover `null`, which is a whole document of the
/// wrong type — hence the hand-written [`Deserialize`] below, via
/// `Option<Wire>`. A module that refused `null` would fail to load for exactly
/// the host that asked nothing of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModuleConfig {
    /// The languages to route, and where.
    pub providers: Vec<ProviderRoute>,
    /// Where worker harnesses are written, or empty for a directory under the
    /// platform cache.
    pub harness_dir: String,
}

/// The fields as they appear on the wire.
///
/// Separate from [`ModuleConfig`] so the hand-written deserializer below can
/// derive the field handling rather than restate it.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Wire {
    providers: Vec<ProviderRoute>,
    harness_dir: String,
}

impl<'de> Deserialize<'de> for ModuleConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // `Option` is what turns `null` into "nothing configured" rather than
        // a type error. Everything else decodes through the derived impl.
        match Option::<Wire>::deserialize(deserializer)? {
            Some(wire) => Ok(Self {
                providers: wire.providers,
                harness_dir: wire.harness_dir,
            }),
            None => Ok(Self::default()),
        }
    }
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

#[cfg(test)]
mod repro_test {
    use super::ModuleConfig;

    #[test]
    fn null_config_decodes() {
        let result: Result<ModuleConfig, _> = serde_json::from_slice(b"null");
        assert!(result.is_ok(), "{result:?}");
    }
}
