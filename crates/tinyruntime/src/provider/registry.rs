//! The routing table: which language goes to which provider.
//!
//! A flat, ordered list rather than a map, because the number of languages is
//! small, the order is what `Languages` reports, and a list keeps registration
//! order meaningful. Lookup is by normalised [`Language`], so a host that spells
//! `"NodeJS"` reaches the same provider as one that spells `"nodejs"`.
//!
//! The registry is built once at module setup from the module's configuration
//! and never mutated afterwards. Adding a language means loading another provider
//! module and naming it in that configuration — not recompiling this crate.

use std::sync::Arc;

use tinyruntime_bus::{Language, LanguageStatus};

use super::{Provider, Route};
use crate::error::{Error, Result};

/// Every language this router can route to.
#[derive(Debug, Default, Clone)]
pub struct Registry {
    routes: Vec<Route>,
}

impl Registry {
    /// An empty registry, which routes nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `provider` as the handler for `language`.
    ///
    /// Re-registering a language replaces its route. A configuration that names
    /// one language twice means the later entry, which is what an operator
    /// overriding a default expects.
    pub fn register(
        &mut self,
        language: Language,
        bus_name: impl Into<String>,
        provider: Arc<dyn Provider>,
    ) {
        let route = Route {
            language: language.clone(),
            bus_name: bus_name.into(),
            provider,
        };
        match self
            .routes
            .iter_mut()
            .find(|existing| existing.language == language)
        {
            Some(existing) => *existing = route,
            None => self.routes.push(route),
        }
    }

    /// The provider registered for `language`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LanguageMissing`] when the request named no language, and
    /// [`Error::UnknownLanguage`] when nothing is registered under it.
    pub fn provider(&self, language: &Language) -> Result<Arc<dyn Provider>> {
        if language.is_empty() {
            return Err(Error::LanguageMissing);
        }
        self.routes
            .iter()
            .find(|route| &route.language == language)
            .map(|route| Arc::clone(&route.provider))
            .ok_or_else(|| Error::UnknownLanguage(language.clone()))
    }

    /// Every registered language, in registration order.
    #[must_use]
    pub fn languages(&self) -> Vec<Language> {
        self.routes
            .iter()
            .map(|route| route.language.clone())
            .collect()
    }

    /// How many languages are registered.
    #[must_use]
    pub fn len(&self) -> usize {
        self.routes.len()
    }

    /// Whether nothing is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.routes.is_empty()
    }

    /// Ask every provider whether it is serving.
    ///
    /// Sequential rather than concurrent on purpose: this is an operator-facing
    /// listing over a handful of entries, and a fan-out that multiplies a slow
    /// provider's timeout across the bus is not worth the milliseconds.
    pub async fn statuses(&self) -> Vec<LanguageStatus> {
        let mut statuses = Vec::with_capacity(self.routes.len());
        for route in &self.routes {
            statuses.push(route.status().await);
        }
        statuses
    }
}
