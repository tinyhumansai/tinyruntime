//! A provider that lives in another module, reached over the bus.
//!
//! This is where routing actually happens: the router holds one of these per
//! language, and a call on it becomes a call on that language's module. Every
//! method is the same three lines — build a proxy, call a member, map the
//! failure — because the interesting part is the contract, not the plumbing.
//!
//! A failure here is always [`Error::ProviderUnavailable`] rather than the bus
//! error verbatim. A host reading `Languages` wants to know that Python is not
//! serving; it does not want a transport error's rendering of a well-known name
//! it never chose.

use tinybus::Connection;

use tinyruntime_bus::{
    Distribution, Language, LayoutRequest, LayoutResponse, ProviderDescriptor, RuntimeLayout,
    RuntimeSettings, WorkerHarness, names,
};

use super::Provider;
use crate::error::{Error, Result};

/// A language provider served by another module on the bus.
#[derive(Clone)]
pub struct BusProvider {
    connection: Connection,
    language: Language,
    bus_name: String,
    /// Where that module's object is, derived from its bus name.
    ///
    /// Derived rather than configured, because it is not independently
    /// choosable: `tinybus_module!` builds a module's manifest path this way, so
    /// this is the only path the provider can be serving at.
    object_path: String,
}

impl std::fmt::Debug for BusProvider {
    /// A connection has no `Debug` of its own, and what identifies this provider
    /// is where it routes rather than which peer it routes through.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BusProvider")
            .field("language", &self.language)
            .field("bus_name", &self.bus_name)
            .finish_non_exhaustive()
    }
}

impl BusProvider {
    /// Route calls for `language` to the module claiming `bus_name`.
    #[must_use]
    pub fn new(connection: Connection, language: Language, bus_name: impl Into<String>) -> Self {
        let bus_name = bus_name.into();
        let object_path = names::object_path_for(&bus_name);
        Self {
            connection,
            language,
            bus_name,
            object_path,
        }
    }

    /// Call `member` on the remote provider with `arguments`.
    ///
    /// The proxy is built per call rather than held: a provider module can be
    /// loaded, or reloaded, after the router started, and a proxy captured at
    /// registration would keep pointing at whatever was there then.
    async fn call<A, R>(&self, member: &str, arguments: A) -> Result<R>
    where
        A: serde::Serialize + Send,
        R: serde::de::DeserializeOwned,
    {
        let proxy = self
            .connection
            .proxy(
                self.bus_name.as_str(),
                self.object_path.as_str(),
                names::PROVIDER_INTERFACE,
            )
            .map_err(|error| self.unavailable(&error))?;

        proxy
            .call(member, arguments)
            .await
            .map_err(|error| self.unavailable(&error))
    }

    /// Render a bus failure as this language's provider being unavailable.
    fn unavailable(&self, error: &tinybus::Error) -> Error {
        tracing::debug!(
            language = self.language.as_str(),
            "[tinyruntime::provider] provider call failed: {error}"
        );
        Error::ProviderUnavailable {
            language: self.language.clone(),
            reason: error.to_string(),
        }
    }
}

#[async_trait::async_trait]
impl Provider for BusProvider {
    async fn describe(&self) -> Result<ProviderDescriptor> {
        self.call(names::provider_methods::DESCRIBE, ()).await
    }

    async fn detect_system(&self, settings: &RuntimeSettings) -> Result<Option<RuntimeLayout>> {
        let response: LayoutResponse = self
            .call(names::provider_methods::DETECT_SYSTEM, (settings,))
            .await?;
        Ok(response.layout)
    }

    async fn select_distribution(&self, settings: &RuntimeSettings) -> Result<Distribution> {
        self.call(names::provider_methods::SELECT_DISTRIBUTION, (settings,))
            .await
    }

    async fn layout(
        &self,
        install_dir: &str,
        settings: &RuntimeSettings,
    ) -> Result<Option<RuntimeLayout>> {
        let response: LayoutResponse = self
            .call(
                names::provider_methods::LAYOUT,
                (LayoutRequest::new(install_dir, settings.clone()),),
            )
            .await?;
        Ok(response.layout)
    }

    async fn harness(&self) -> Result<WorkerHarness> {
        self.call(names::provider_methods::HARNESS, ()).await
    }
}

#[cfg(test)]
#[path = "bus_test.rs"]
mod test;
