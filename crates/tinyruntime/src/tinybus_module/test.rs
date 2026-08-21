//! Tests for the module adapter and for routing over a real bus.
//!
//! The interesting one is [`resolve_routes_to_a_provider_module`]: a second peer
//! on the same bus serves the provider interface, and the router reaches it for
//! an answer it could not have produced itself. That is the whole design working
//! end to end — two modules, one contract, no language knowledge in the router.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Interface, Result as TinyBusResult};

use tinyruntime_bus::{
    Language, LanguagesResponse, LayoutRequest, LayoutResponse, ProviderDescriptor, ResolveRequest,
    ResolveResponse, RuntimeLayout, RuntimeSettings, RuntimeSource, WorkerHarness, names,
};

use super::{RuntimeService, setup};
use crate::config::{ModuleConfig, ProviderRoute};
use crate::exec::Engine;
use crate::provider::Registry;

/// A stand-in for `tinyruntime-nodejs`, serving the provider interface.
struct FakeProvider;

#[tinybus::interface(name = "ai.tinyhumans.runtime.Provider")]
impl FakeProvider {
    async fn describe(&self) -> TinyBusResult<ProviderDescriptor> {
        // The interface macro dispatches futures, so every member is `async`
        // even where the answer is a constant.
        std::future::ready(Ok(ProviderDescriptor::new(
            Language::nodejs(),
            "Fake Node.js",
            "1.0.0",
        )))
        .await
    }

    async fn detect_system(&self, _settings: RuntimeSettings) -> TinyBusResult<LayoutResponse> {
        std::future::ready(Ok(LayoutResponse::found(
            RuntimeLayout::new("1.2.3", "/usr/local/bin")
                .with_executable("node", "/usr/local/bin/node"),
        )))
        .await
    }

    async fn layout(&self, _request: LayoutRequest) -> TinyBusResult<LayoutResponse> {
        std::future::ready(Ok(LayoutResponse::missing())).await
    }

    async fn harness(&self) -> TinyBusResult<WorkerHarness> {
        std::future::ready(Ok(WorkerHarness::new(
            "pool_worker.js",
            "// harness",
            "node",
        )))
        .await
    }
}

/// The bus name the fake provider claims in these tests.
const FAKE_BUS_NAME: &str = "ai.tinyhumans.runtime.nodejs.Provider";

/// Start a broker and return a bus every peer in a test connects through.
fn bus() -> MemoryBus {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    bus
}

/// A configuration routing only Node.js, at the fake provider's bus name.
fn config_routing_node(harness_dir: &std::path::Path) -> ModuleConfig {
    ModuleConfig {
        providers: vec![ProviderRoute::new(Language::nodejs(), FAKE_BUS_NAME)],
        harness_dir: harness_dir.to_string_lossy().into_owned(),
    }
}

#[test]
fn declared_methods_match_the_dispatch_table() {
    let service = RuntimeService {
        engine: std::sync::Arc::new(Engine::new(
            Registry::new(),
            reqwest::Client::new(),
            std::path::PathBuf::from("/tmp"),
        )),
    };
    let methods = service
        .members()
        .into_iter()
        .map(|member| member.to_string())
        .collect::<Vec<_>>();

    assert_eq!(methods, names::METHODS.to_vec());
}

#[test]
fn the_served_interface_name_matches_the_contract() {
    let service = RuntimeService {
        engine: std::sync::Arc::new(Engine::new(
            Registry::new(),
            reqwest::Client::new(),
            std::path::PathBuf::from("/tmp"),
        )),
    };
    assert_eq!(service.name().to_string(), names::INTERFACE);
}

#[test]
fn the_fake_provider_serves_the_provider_interface_from_the_contract() {
    // If this drifts, every routing test below would be exercising an interface
    // no real provider implements.
    assert_eq!(FakeProvider.name().to_string(), names::PROVIDER_INTERFACE);
}

#[tokio::test]
async fn a_language_whose_provider_is_absent_is_listed_as_unavailable() -> TinyBusResult<()> {
    // One missing provider must not take down the listing, or the host cannot
    // tell which languages it *can* use.
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();
    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: LanguagesResponse = proxy.call(names::methods::LANGUAGES, ()).await?;

    assert_eq!(reply.languages.len(), 1);
    assert!(!reply.languages[0].available);
    assert!(reply.languages[0].detail.is_some());
    Ok(())
}

#[tokio::test]
async fn a_provider_module_on_the_bus_is_listed_as_available() -> TinyBusResult<()> {
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();

    let provider = Connection::connect(bus.connect().await?).await?;
    provider
        .serve_at(names::PROVIDER_OBJECT_PATH.try_into()?, FakeProvider)
        .await?;
    provider.request_name(FAKE_BUS_NAME).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: LanguagesResponse = proxy.call(names::methods::LANGUAGES, ()).await?;

    assert!(
        reply.languages[0].available,
        "detail: {:?}",
        reply.languages[0].detail
    );
    assert_eq!(
        reply.languages[0].display_name.as_deref(),
        Some("Fake Node.js")
    );
    Ok(())
}

#[tokio::test]
async fn resolve_routes_to_a_provider_module() -> TinyBusResult<()> {
    // The router has no idea what Node.js is. Everything in this answer came
    // from the other peer.
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();

    let provider = Connection::connect(bus.connect().await?).await?;
    provider
        .serve_at(names::PROVIDER_OBJECT_PATH.try_into()?, FakeProvider)
        .await?;
    provider.request_name(FAKE_BUS_NAME).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    let mut settings = RuntimeSettings::new("1.0.0");
    settings.cache_dir = scratch.path().to_string_lossy().into_owned();
    let reply: ResolveResponse = proxy
        .call(
            names::methods::RESOLVE,
            (ResolveRequest::probe(Language::nodejs(), settings),),
        )
        .await?;

    let runtime = reply
        .runtime
        .expect("the provider reported a host toolchain");
    assert_eq!(runtime.version, "1.2.3");
    assert_eq!(runtime.source, RuntimeSource::System);
    assert_eq!(runtime.executable("node"), Some("/usr/local/bin/node"));
    Ok(())
}

#[tokio::test]
async fn resolving_an_unrouted_language_fails_with_a_readable_reason() -> TinyBusResult<()> {
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();
    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let result = proxy
        .call::<ResolveResponse>(
            names::methods::RESOLVE,
            (ResolveRequest::probe(
                Language::python(),
                RuntimeSettings::new("3.12"),
            ),),
        )
        .await;

    let Err(error) = result else {
        return Err(tinybus::Error::failed(
            "an unrouted language unexpectedly resolved",
        ));
    };
    let rendered = error.to_string();
    assert!(rendered.contains("python"), "got `{rendered}`");
    assert!(
        !rendered.contains('/'),
        "the error leaked a path: `{rendered}`"
    );
    Ok(())
}

#[tokio::test]
async fn pool_stats_are_empty_before_anything_runs() -> TinyBusResult<()> {
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();
    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: tinyruntime_bus::PoolStatsResponse =
        proxy.call(names::methods::POOL_STATS, ()).await?;

    assert!(reply.pools.is_empty(), "a pool existed before any job ran");
    Ok(())
}
