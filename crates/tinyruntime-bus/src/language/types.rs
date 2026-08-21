//! The language identifier that selects a runtime provider.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Which language runtime a request is about.
///
/// This is the routing key: the router keeps one provider per [`Language`], and
/// every request carries the language it is for. It is a newtype over a string
/// rather than an enum on purpose — a new language is a new provider module, and
/// adding one must not require recompiling the router or the contract.
///
/// # Examples
///
/// ```
/// # use tinyruntime_bus::Language;
/// assert_eq!(Language::NODEJS.as_str(), "nodejs");
/// assert_eq!(Language::new("Python").as_str(), "python");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Language(String);

impl Language {
    /// JavaScript on the managed Node.js toolchain.
    pub const NODEJS: Self = Self(String::new());

    /// Builds a language identifier, lowercased and trimmed.
    ///
    /// Normalising here is what lets a host spell `"Node.js"`, `"NODEJS"`, or
    /// `"nodejs"` and still reach the same provider.
    #[must_use]
    pub fn new(id: impl AsRef<str>) -> Self {
        Self(id.as_ref().trim().to_ascii_lowercase())
    }

    /// The normalised identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        if self.0.is_empty() { NODEJS_ID } else { &self.0 }
    }

    /// Whether this identifier names no language at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl From<&str> for Language {
    fn from(id: &str) -> Self {
        Self::new(id)
    }
}

impl From<String> for Language {
    fn from(id: String) -> Self {
        Self::new(id)
    }
}

/// The identifier the Node.js provider answers to.
pub const NODEJS_ID: &str = "nodejs";

/// The identifier the Python provider answers to.
pub const PYTHON_ID: &str = "python";
