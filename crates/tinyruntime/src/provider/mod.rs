//! What a language provider can be asked, and how the router finds one.
//!
//! [`Provider`] is the whole language-specific surface of this system: five
//! questions, none of which touch the network for bytes, unpack anything, or
//! spawn a worker. A provider that answers them correctly gets the router's
//! entire download, verification, install, reuse, and pooling pipeline for free,
//! and a provider that would like to reimplement any of that cannot.
//!
//! The trait is deliberately not tied to the bus. [`BusProvider`] is one
//! implementation — the one that routes to a sibling module — but the router
//! itself only ever sees the trait, which is what lets the resolution and
//! execution paths be tested against a provider that answers from memory instead
//! of from a release channel and a running module.

use std::sync::Arc;

use tinyruntime_bus::{
    Distribution, Language, LanguageStatus, ProviderDescriptor, RuntimeLayout, RuntimeSettings,
    WorkerHarness,
};

use crate::error::{Error, Result};

mod bus;
mod registry;
#[cfg(test)]
pub(crate) mod stub;

pub use bus::BusProvider;
pub use registry::Registry;

/// The five questions only a language module can answer.
///
/// Everything a provider returns is a description. Nothing it returns is a side
/// effect: the router downloads what [`Provider::select_distribution`] names,
/// installs it where the router decides, and launches what
/// [`Provider::harness`] supplies.
#[async_trait::async_trait]
pub trait Provider: std::fmt::Debug + Send + Sync {
    /// What this provider is and what it targets by default.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProviderUnavailable`] when the provider cannot be
    /// reached.
    async fn describe(&self) -> Result<ProviderDescriptor>;

    /// Look for a compatible toolchain already on the host.
    ///
    /// Returning `None` means the host has nothing suitable, which is an
    /// ordinary answer — the router then installs a managed toolchain.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProviderUnavailable`] when the provider cannot be
    /// reached.
    async fn detect_system(&self, settings: &RuntimeSettings) -> Result<Option<RuntimeLayout>>;

    /// Pick the archive to install for this host and these settings.
    ///
    /// This is the only provider method allowed to reach the network, and only
    /// to read a release index — never to fetch the archive itself.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProviderUnavailable`] when the provider cannot be
    /// reached, or [`Error::Download`] when it cannot read its release index.
    async fn select_distribution(&self, settings: &RuntimeSettings) -> Result<Distribution>;

    /// Report where the executables are inside an extracted install.
    ///
    /// Returning `None` means the directory holds no toolchain these settings
    /// accept, which the router treats as "not installed" rather than as a
    /// failure. The settings are what let a provider decline an install that is
    /// real but wrong — a Python 3.11 tree for a request that needs 3.12.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProviderUnavailable`] when the provider cannot be
    /// reached.
    async fn layout(
        &self,
        install_dir: &str,
        settings: &RuntimeSettings,
    ) -> Result<Option<RuntimeLayout>>;

    /// Supply the worker harness the router launches for this language.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ProviderUnavailable`] when the provider cannot be
    /// reached.
    async fn harness(&self) -> Result<WorkerHarness>;
}

/// One language the router can route to.
#[derive(Clone)]
pub struct Route {
    /// The routing key.
    pub language: Language,
    /// The well-known bus name the provider is expected to claim, for reporting.
    pub bus_name: String,
    /// The provider itself.
    pub provider: Arc<dyn Provider>,
}

impl std::fmt::Debug for Route {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Route")
            .field("language", &self.language)
            .field("bus_name", &self.bus_name)
            .finish_non_exhaustive()
    }
}

impl Route {
    /// Ask this route's provider whether it is serving, and whether this build
    /// can bind to the contract it reports.
    ///
    /// Never fails: a provider that is down or incompatible is a language that is
    /// unavailable, and the caller of `Languages` wants that as a listed row with
    /// a reason rather than as an error that hides every other language too.
    pub async fn status(&self) -> LanguageStatus {
        match self.provider.describe().await {
            Ok(descriptor) if tinyruntime_bus::is_compatible(descriptor.contract_version) => {
                LanguageStatus {
                    language: self.language.clone(),
                    bus_name: self.bus_name.clone(),
                    available: true,
                    display_name: Some(descriptor.display_name),
                    detail: None,
                }
            }
            Ok(descriptor) => {
                let (major, minor) = descriptor.contract_version;
                LanguageStatus {
                    language: self.language.clone(),
                    bus_name: self.bus_name.clone(),
                    available: false,
                    display_name: Some(descriptor.display_name),
                    detail: Some(format!(
                        "the provider speaks contract {major}.{minor}, which this build cannot bind to"
                    )),
                }
            }
            Err(error) => LanguageStatus {
                language: self.language.clone(),
                bus_name: self.bus_name.clone(),
                available: false,
                display_name: None,
                detail: Some(error.to_string()),
            },
        }
    }
}

/// Refuse a provider whose contract this build cannot bind to.
///
/// Checked once per resolution rather than once per call: a provider that
/// answers a question the router did not ask is a worse outcome than a language
/// that is simply unavailable, and finding out halfway through an install is
/// worse still.
///
/// # Errors
///
/// Returns [`Error::ProviderContract`] when the versions cannot bind.
pub(crate) fn verify_contract(
    language: &Language,
    descriptor: &ProviderDescriptor,
) -> Result<()> {
    if tinyruntime_bus::is_compatible(descriptor.contract_version) {
        return Ok(());
    }
    let (major, minor) = descriptor.contract_version;
    Err(Error::ProviderContract {
        language: language.clone(),
        major,
        minor,
    })
}

#[cfg(test)]
mod test;
