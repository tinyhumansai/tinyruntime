//! Turning a request for a language into a toolchain that can run code.
//!
//! The order matters more than any single step, because each step exists to
//! avoid the cost of the next one:
//!
//! 1. **A resolution this process already made.** Free.
//! 2. **A compatible toolchain on the host.** One `--version` probe, and the
//!    common developer machine never downloads anything at all.
//! 3. **A managed toolchain already in the cache.** A few `stat`s, and a warm
//!    restart after a previous install costs nothing. This is the step that makes
//!    "reuse" real rather than aspirational — without it every process start pays
//!    for the network again.
//! 4. **A managed toolchain this call installs.** Hundreds of megabytes, once.
//!
//! Steps 1 to 3 never touch the network, which is what makes a non-installing
//! probe a useful thing for a host to call: it answers "is this ready?" without
//! committing anyone to a download.

use std::collections::HashMap;
use std::path::Path;

use reqwest::Client;
use tokio::sync::Mutex;

use tinyruntime_bus::{
    Language, ResolveRequest, ResolvedRuntime, RuntimeLayout, RuntimeSettings, RuntimeSource,
};

use crate::error::{Error, Result};
use crate::provider::{Provider, Registry, verify_contract};
use crate::store;

mod install;
mod reuse;

/// Resolves toolchains, and remembers what it resolved.
#[derive(Debug)]
pub struct Resolver {
    registry: Registry,
    client: Client,
    /// One entry per (language, settings) pair already resolved in this process.
    ///
    /// Keyed by the settings as well as the language because two callers may
    /// legitimately want different versions of the same language, and answering
    /// the second from the first's memo would silently hand it the wrong one.
    resolved: Mutex<HashMap<String, ResolvedRuntime>>,
}

impl Resolver {
    /// Build a resolver over `registry`.
    #[must_use]
    pub fn new(registry: Registry, client: Client) -> Self {
        Self {
            registry,
            client,
            resolved: Mutex::new(HashMap::new()),
        }
    }

    /// The routing table this resolver was built over.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Resolve `request`, installing a managed toolchain when it allows one.
    ///
    /// Returns `None` only for a non-installing request that found nothing
    /// provisioned. A request that allows installing either produces a toolchain
    /// or fails saying why.
    ///
    /// # Errors
    ///
    /// Returns [`Error::LanguageDisabled`] when the host has the language turned
    /// off, [`Error::UnknownLanguage`] when nothing is registered for it,
    /// [`Error::ProviderUnavailable`] or [`Error::ProviderContract`] when its
    /// provider cannot serve this build, and the download and install variants
    /// when provisioning fails.
    pub async fn resolve(&self, request: &ResolveRequest) -> Result<Option<ResolvedRuntime>> {
        let language = &request.language;
        let settings = &request.settings;

        if !settings.enabled {
            return Err(Error::LanguageDisabled(language.clone()));
        }

        let key = memo_key(language, settings);
        if let Some(existing) = self.resolved.lock().await.get(&key).cloned() {
            tracing::debug!(
                language = language.as_str(),
                "[tinyruntime::resolve] reusing a resolution from this process"
            );
            return Ok(Some(existing));
        }

        let provider = self.registry.provider(language)?;
        verify_contract(language, &provider.describe().await?)?;

        if let Some(resolved) = self
            .without_network(language, settings, provider.as_ref())
            .await?
        {
            self.remember(key, resolved.clone()).await;
            return Ok(Some(resolved));
        }

        if !request.install {
            tracing::debug!(
                language = language.as_str(),
                "[tinyruntime::resolve] nothing provisioned and this request forbade installing"
            );
            return Ok(None);
        }

        let resolved = self.install(language, settings, provider.as_ref()).await?;
        self.remember(key, resolved.clone()).await;
        Ok(Some(resolved))
    }

    /// Resolve `request`, failing rather than answering "nothing provisioned".
    ///
    /// The shape callers who are about to *run* something want: they cannot do
    /// anything useful with `None`, so turning it into an error here keeps that
    /// unwrapping out of every call site.
    ///
    /// # Errors
    ///
    /// As [`Resolver::resolve`], plus [`Error::NotProvisioned`] when a
    /// non-installing request found nothing.
    pub async fn require(&self, request: &ResolveRequest) -> Result<ResolvedRuntime> {
        self.resolve(request)
            .await?
            .ok_or_else(|| Error::NotProvisioned(request.language.clone()))
    }

    /// Steps 2 and 3: everything that can be answered without the network.
    async fn without_network(
        &self,
        language: &Language,
        settings: &RuntimeSettings,
        provider: &dyn Provider,
    ) -> Result<Option<ResolvedRuntime>> {
        if settings.prefer_system {
            if let Some(layout) = provider.detect_system(settings).await? {
                tracing::info!(
                    language = language.as_str(),
                    version = %layout.version,
                    "[tinyruntime::resolve] reusing a compatible toolchain from the host"
                );
                return Ok(Some(ResolvedRuntime::from_layout(
                    language.clone(),
                    RuntimeSource::System,
                    layout,
                )));
            }
        }

        let root = store::cache_root(settings.cache_dir(), language);
        Ok(reuse::scan(&root, language, settings, provider).await)
    }

    /// Step 4: download, verify, unpack, and promote a managed toolchain.
    async fn install(
        &self,
        language: &Language,
        settings: &RuntimeSettings,
        provider: &dyn Provider,
    ) -> Result<ResolvedRuntime> {
        install::run(&self.client, language, settings, provider).await
    }

    /// Record a resolution so the next identical request is free.
    async fn remember(&self, key: String, resolved: ResolvedRuntime) {
        self.resolved.lock().await.insert(key, resolved);
    }
}

/// The memo key for one language under one set of settings.
///
/// Only the fields that can change *which* toolchain is resolved take part. The
/// pool tuning is not here because it does not, and including it would throw
/// away a perfectly good resolution every time a host retuned its worker count.
fn memo_key(language: &Language, settings: &RuntimeSettings) -> String {
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        language.as_str(),
        settings.version,
        settings.maximum_version,
        settings.cache_dir,
        settings.release_tag,
        settings.preferred_command,
        settings.prefer_system,
    )
}

/// Build a resolution from a layout found in `install_dir`.
fn managed(language: &Language, install_dir: &Path, layout: RuntimeLayout) -> ResolvedRuntime {
    ResolvedRuntime::from_layout(language.clone(), RuntimeSource::Managed, layout)
        .with_install_dir(install_dir.to_string_lossy().into_owned())
}

#[cfg(test)]
mod test;
