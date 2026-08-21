//! The `TinyBus` module entrypoint and the router's bus-facing interface.
//!
//! This adapter is the only part of the crate that knows about `TinyBus`.
//! Everything below it — [`Engine`], the resolver, the pools — is ordinary Rust
//! that can be built and tested without a bus, which is what keeps the
//! interesting behaviour testable and this file boring.
//!
//! The names and payload types come from [`tinyruntime_bus`], so a host spells
//! them from the contract crate rather than repeating string literals.
//!
//! # Setup
//!
//! Setup does one interesting thing: it builds the routing table from the
//! module's configuration, wrapping each configured bus name in a
//! [`BusProvider`] over this module's own connection. That connection is what
//! makes routing possible — the router is a bus *client* as well as a server,
//! and a call it receives becomes a call it makes.
//!
//! Nothing is contacted at setup. A provider module may be loaded after this one,
//! or never; a language whose provider is absent reports itself unavailable when
//! asked rather than preventing this module from serving the ones that are there.

use std::sync::Arc;

use tinybus::{Connection, Result as TinyBusResult};

use tinyruntime_bus::{
    ExecRequest, ExecResponse, LanguagesResponse, PoolStatsResponse, ResolveRequest,
    ResolveResponse, names,
};

use crate::config::ModuleConfig;
use crate::error::Error;
use crate::exec::Engine;
use crate::provider::{BusProvider, Registry};

/// The object this module serves.
struct RuntimeService {
    engine: Arc<Engine>,
}

#[tinybus::interface(name = "ai.tinyhumans.runtime.Runtime")]
impl RuntimeService {
    /// Every language this router can route to, and whether it currently can.
    async fn languages(&self) -> TinyBusResult<LanguagesResponse> {
        Ok(LanguagesResponse::new(
            self.engine.registry().statuses().await,
        ))
    }

    /// Resolve a language runtime, installing it when the request allows.
    async fn resolve(&self, request: ResolveRequest) -> TinyBusResult<ResolveResponse> {
        match self.engine.resolve(&request).await {
            Ok(Some(runtime)) => Ok(ResolveResponse::found(runtime)),
            Ok(None) => Ok(ResolveResponse::missing()),
            Err(error) => Err(failed(&error)),
        }
    }

    /// Run inline source on a language runtime, resolving it first.
    async fn execute(&self, request: ExecRequest) -> TinyBusResult<ExecResponse> {
        self.engine
            .execute(&request)
            .await
            .map_err(|error| failed(&error))
    }

    /// Every live worker pool's counters.
    async fn pool_stats(&self) -> TinyBusResult<PoolStatsResponse> {
        Ok(PoolStatsResponse::new(self.engine.pool_stats().await))
    }
}

/// Render a router failure as a bus error.
///
/// Every variant's `Display` is already written for this: lowercase, no
/// credential, no payload, no absolute path. A host renders what comes back.
fn failed(error: &Error) -> tinybus::Error {
    tinybus::Error::failed(error.to_string())
}

/// Build the routing table and start serving.
async fn setup(connection: Connection, config: ModuleConfig) -> TinyBusResult<()> {
    let mut registry = Registry::new();
    for route in &config.providers {
        registry.register(
            &route.language,
            route.bus_name.clone(),
            Arc::new(BusProvider::new(
                connection.clone(),
                route.language.clone(),
                route.bus_name.clone(),
            )),
        );
    }
    tracing::info!(
        languages = registry.len(),
        "[tinyruntime] routing table built"
    );

    let engine = Arc::new(Engine::new(
        registry,
        reqwest::Client::new(),
        config.harness_root(),
    ));

    connection
        .serve_at(names::OBJECT_PATH.try_into()?, RuntimeService { engine })
        .await?;
    connection.request_name(names::INTERFACE).await?;
    Ok(())
}

tinybus_module::module_export! {
    setup = setup,
    config = ModuleConfig,
    worker_threads = 2,
    provides = ["ai.tinyhumans.runtime.Runtime"],
    methods = ["Languages", "Resolve", "Execute", "PoolStats"],
    signals = [],
    requires = [],
    optional = ["ai.tinyhumans.runtime.Provider"],
    lazy = false,
}

#[cfg(test)]
mod test;
