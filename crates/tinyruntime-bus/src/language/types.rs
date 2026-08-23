//! The language identifier that selects a runtime provider.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize};

/// The identifier the Node.js provider answers to.
pub const NODEJS: &str = "nodejs";

/// The identifier the Python provider answers to.
pub const PYTHON: &str = "python";

/// Which language runtime a request is about.
///
/// This is the routing key: the router keeps one provider per [`Language`], and
/// every request carries the language it is for. It is a newtype over a string
/// rather than an enum on purpose — a new language is a new provider module, and
/// adding one must not mean recompiling the router or this contract.
///
/// # Examples
///
/// ```
/// # use tinyruntime_bus::Language;
/// assert_eq!(Language::nodejs().as_str(), "nodejs");
/// assert_eq!(Language::new(" Python ").as_str(), "python");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct Language(String);

/// Decoding normalises exactly as [`Language::new`] does.
///
/// Deriving this would not: a derived `transparent` implementation hands the raw
/// string straight to the newtype, so a peer that spelled `"NodeJS"` on the wire
/// would arrive as an identifier no provider is registered under and fail to
/// route. Normalising on the way in is what makes the constructor's promise hold
/// for values that never went through the constructor.
impl<'de> Deserialize<'de> for Language {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

impl Language {
    /// Builds a language identifier, trimmed and lowercased.
    ///
    /// Normalising here is what lets a host spell `"NodeJS"`, `"NODEJS"`, or
    /// `"nodejs"` and still reach the same provider.
    #[must_use]
    pub fn new(id: impl AsRef<str>) -> Self {
        Self(id.as_ref().trim().to_ascii_lowercase())
    }

    /// The Node.js language.
    #[must_use]
    pub fn nodejs() -> Self {
        Self(NODEJS.to_owned())
    }

    /// The Python language.
    #[must_use]
    pub fn python() -> Self {
        Self(PYTHON.to_owned())
    }

    /// The normalised identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Whether this identifier names no language at all.
    ///
    /// A request carrying an empty language is rejected rather than routed to a
    /// default: guessing which runtime the caller meant is worse than an error.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Display for Language {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
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
