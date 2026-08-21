//! A provider that answers from memory, for testing the router without a bus.
//!
//! The router's interesting behaviour — reuse before download, install under a
//! lock, promote atomically, keep a worker warm — is all language-agnostic, so
//! testing it should not require a second module, a release channel, or a
//! network. This stub answers the five provider questions from fields a test
//! sets, and records what it was asked.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tinyruntime_bus::{
    Distribution, Language, ProviderDescriptor, RuntimeLayout, RuntimeSettings, WorkerHarness,
};

use super::Provider;
use crate::error::{Error, Result};

/// A provider whose answers a test supplies up front.
#[derive(Debug)]
pub(crate) struct StubProvider {
    language: Language,
    descriptor: ProviderDescriptor,
    system: Mutex<Option<RuntimeLayout>>,
    distribution: Mutex<Option<Distribution>>,
    layout: Mutex<Option<RuntimeLayout>>,
    /// A path, relative to an install directory, that must exist before
    /// `layout` reports anything.
    ///
    /// Without this the stub calls every directory a toolchain, including one
    /// the router has not installed into yet — which silently turns an install
    /// test into a no-op.
    layout_marker: Mutex<Option<String>>,
    harness: Mutex<Option<WorkerHarness>>,
    /// How many times a distribution was selected, which is how a test sees
    /// whether the reuse path avoided a download.
    pub(crate) selections: AtomicUsize,
    /// How many times the host was probed for an existing toolchain.
    pub(crate) detections: AtomicUsize,
}

impl StubProvider {
    /// A stub for `language` that finds nothing on the host and offers nothing
    /// to install.
    pub(crate) fn new(language: Language) -> Self {
        let descriptor = ProviderDescriptor::new(language.clone(), "Stub", "1.0.0");
        Self {
            language,
            descriptor,
            system: Mutex::new(None),
            distribution: Mutex::new(None),
            layout: Mutex::new(None),
            layout_marker: Mutex::new(None),
            harness: Mutex::new(None),
            selections: AtomicUsize::new(0),
            detections: AtomicUsize::new(0),
        }
    }

    /// Report `layout` as a compatible toolchain already on the host.
    pub(crate) fn with_system(self, layout: RuntimeLayout) -> Self {
        *self.system.lock().expect("uncontended in tests") = Some(layout);
        self
    }

    /// Offer `distribution` when the router asks what to install.
    pub(crate) fn with_distribution(self, distribution: Distribution) -> Self {
        *self.distribution.lock().expect("uncontended in tests") = Some(distribution);
        self
    }

    /// Report `layout` for any install directory the router asks about.
    pub(crate) fn with_layout(self, layout: RuntimeLayout) -> Self {
        *self.layout.lock().expect("uncontended in tests") = Some(layout);
        self
    }

    /// Report `layout`, but only for a directory that actually contains
    /// `marker` — which is what a real provider does.
    pub(crate) fn with_layout_when_present(
        self,
        marker: impl Into<String>,
        layout: RuntimeLayout,
    ) -> Self {
        *self.layout_marker.lock().expect("uncontended in tests") = Some(marker.into());
        self.with_layout(layout)
    }

    /// Supply `harness` when the router asks how to launch a worker.
    pub(crate) fn with_harness(self, harness: WorkerHarness) -> Self {
        *self.harness.lock().expect("uncontended in tests") = Some(harness);
        self
    }

    /// Report an incompatible contract version, as a provider from the future
    /// would.
    pub(crate) fn with_contract(mut self, version: (u32, u32)) -> Self {
        self.descriptor.contract_version = version;
        self
    }
}

#[async_trait::async_trait]
impl Provider for StubProvider {
    async fn describe(&self) -> Result<ProviderDescriptor> {
        Ok(self.descriptor.clone())
    }

    async fn detect_system(&self, _settings: &RuntimeSettings) -> Result<Option<RuntimeLayout>> {
        self.detections.fetch_add(1, Ordering::Relaxed);
        Ok(self.system.lock().expect("uncontended in tests").clone())
    }

    async fn select_distribution(&self, _settings: &RuntimeSettings) -> Result<Distribution> {
        self.selections.fetch_add(1, Ordering::Relaxed);
        self.distribution
            .lock()
            .expect("uncontended in tests")
            .clone()
            .ok_or_else(|| Error::Download {
                language: self.language.clone(),
                reason: "the stub was given nothing to install".to_string(),
            })
    }

    async fn layout(
        &self,
        install_dir: &str,
        _settings: &RuntimeSettings,
    ) -> Result<Option<RuntimeLayout>> {
        if let Some(marker) = self
            .layout_marker
            .lock()
            .expect("uncontended in tests")
            .as_ref()
            && !std::path::Path::new(install_dir).join(marker).exists()
        {
            return Ok(None);
        }
        Ok(self.layout.lock().expect("uncontended in tests").clone())
    }

    async fn harness(&self) -> Result<WorkerHarness> {
        self.harness
            .lock()
            .expect("uncontended in tests")
            .clone()
            .ok_or_else(|| Error::ProviderUnavailable {
                language: self.language.clone(),
                reason: "the stub was given no harness".to_string(),
            })
    }
}

/// A provider that is registered but never answers.
#[derive(Debug)]
pub(crate) struct DownProvider(pub(crate) Language);

#[async_trait::async_trait]
impl Provider for DownProvider {
    async fn describe(&self) -> Result<ProviderDescriptor> {
        Err(self.down())
    }

    async fn detect_system(&self, _settings: &RuntimeSettings) -> Result<Option<RuntimeLayout>> {
        Err(self.down())
    }

    async fn select_distribution(&self, _settings: &RuntimeSettings) -> Result<Distribution> {
        Err(self.down())
    }

    async fn layout(
        &self,
        _install_dir: &str,
        _settings: &RuntimeSettings,
    ) -> Result<Option<RuntimeLayout>> {
        Err(self.down())
    }

    async fn harness(&self) -> Result<WorkerHarness> {
        Err(self.down())
    }
}

impl DownProvider {
    fn down(&self) -> Error {
        Error::ProviderUnavailable {
            language: self.0.clone(),
            reason: "the provider module is not loaded".to_string(),
        }
    }
}
