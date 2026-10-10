//! Tests for routing to a provider over a real bus.
//!
//! [`BusProvider`] is where routing actually happens, so these drive every one
//! of the five members against a peer serving the provider interface, and
//! against a name nobody owns. The in-memory transport is a real broker with
//! real framing — only the sockets are absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Result as TinyBusResult};

use tinyruntime_bus::worker::WorkerCommand;
use tinyruntime_bus::{
    ArchiveFormat, Distribution, EnvironmentPreparationPlan, EnvironmentPreparationRequest,
    Language, LayoutRequest, LayoutResponse, PackageGroup, ProviderCapability, ProviderDescriptor,
    RuntimeLayout, RuntimeSettings, WorkerHarness, names,
};

use super::{BusProvider, Provider};
use crate::Route;
use crate::error::Error;

/// A peer answering all five provider members.
struct Served {
    preparation_capability: bool,
}

#[tinybus::interface(name = "ai.tinyhumans.runtime.Provider")]
impl Served {
    async fn describe(&self) -> TinyBusResult<ProviderDescriptor> {
        let mut descriptor =
            ProviderDescriptor::new(Language::nodejs(), "Served", "1.0.0").with_executable("tool");
        if self.preparation_capability {
            descriptor = descriptor.with_capability(ProviderCapability::PrepareEnvironment);
        }
        std::future::ready(Ok(descriptor)).await
    }

    async fn detect_system(&self, _settings: RuntimeSettings) -> TinyBusResult<LayoutResponse> {
        std::future::ready(Ok(LayoutResponse::found(
            RuntimeLayout::new("1.0.0", "/host/bin").with_executable("tool", "/host/bin/tool"),
        )))
        .await
    }

    async fn select_distribution(&self, _settings: RuntimeSettings) -> TinyBusResult<Distribution> {
        std::future::ready(Ok(Distribution::new(
            "1.0.0",
            "t.tar.gz",
            "https://example.invalid/t.tar.gz",
            ArchiveFormat::TarGz,
        )))
        .await
    }

    async fn layout(&self, request: LayoutRequest) -> TinyBusResult<LayoutResponse> {
        // Echo the directory back through the version, so a test can prove the
        // request actually crossed rather than being answered from a constant.
        std::future::ready(Ok(LayoutResponse::found(RuntimeLayout::new(
            request.install_dir,
            "/installed/bin",
        ))))
        .await
    }

    async fn harness(&self) -> TinyBusResult<WorkerHarness> {
        std::future::ready(Ok(WorkerHarness::new("worker.js", "// body", "tool"))).await
    }

    async fn prepare_environment(
        &self,
        request: EnvironmentPreparationRequest,
    ) -> TinyBusResult<EnvironmentPreparationPlan> {
        std::future::ready(Ok(EnvironmentPreparationPlan {
            executable: format!("{}/bin/python", request.environment_root),
            steps: vec![WorkerCommand {
                executable: request.runtime_executable,
                args: request
                    .package_groups
                    .iter()
                    .flat_map(|group| group.packages.iter().cloned())
                    .collect(),
                env: request
                    .package_groups
                    .first()
                    .and_then(|group| group.source.clone())
                    .map(|source| vec![("SOURCE_HINT".into(), source)])
                    .unwrap_or_default(),
                timeout_ms: Some(1_800_000),
            }],
        }))
        .await
    }
}

/// The bus name the served peer claims in these tests.
const BUS_NAME: &str = names::providers::NODEJS;

/// Start a broker, serve the provider, and return a routed client for it.
async fn routed_with_capability(
    preparation_capability: bool,
) -> TinyBusResult<(Connection, BusProvider)> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let peer = Connection::connect(bus.connect().await?).await?;
    peer.serve_at(
        names::object_path_for(BUS_NAME).as_str().try_into()?,
        Served {
            preparation_capability,
        },
    )
    .await?;
    peer.request_name(BUS_NAME).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    Ok((peer, BusProvider::new(client, Language::nodejs(), BUS_NAME)))
}

async fn routed() -> TinyBusResult<(Connection, BusProvider)> {
    routed_with_capability(false).await
}

/// A client routed at a name nobody owns.
async fn unrouted() -> TinyBusResult<BusProvider> {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let client = Connection::connect(bus.connect().await?).await?;
    Ok(BusProvider::new(
        client,
        Language::python(),
        names::providers::PYTHON,
    ))
}

#[tokio::test]
async fn every_member_reaches_the_provider() -> TinyBusResult<()> {
    // `_peer` is held: dropping it disconnects the provider and takes its name.
    let (_peer, provider) = routed().await?;

    let descriptor = provider.describe().await.expect("describe routes");
    assert_eq!(descriptor.display_name, "Served");

    let system = provider
        .detect_system(&RuntimeSettings::new("1.0.0"))
        .await
        .expect("detect_system routes");
    assert_eq!(system.expect("a host toolchain").version, "1.0.0");

    let distribution = provider
        .select_distribution(&RuntimeSettings::new("1.0.0"))
        .await
        .expect("select_distribution routes");
    assert_eq!(distribution.archive_name, "t.tar.gz");

    let layout = provider
        .layout("/cache/toolchain", &RuntimeSettings::new("1.0.0"))
        .await
        .expect("layout routes");
    assert_eq!(
        layout.expect("a layout").version,
        "/cache/toolchain",
        "the install directory did not cross the bus"
    );

    let harness = provider.harness().await.expect("harness routes");
    assert_eq!(harness.filename, "worker.js");
    Ok(())
}

#[tokio::test]
async fn an_old_five_member_provider_skips_the_unadvertised_optional_operation() -> TinyBusResult<()>
{
    let (_peer, provider) = routed().await?;
    let route = Route {
        language: Language::nodejs(),
        bus_name: BUS_NAME.into(),
        provider: Arc::new(provider),
    };
    let result = route
        .prepare_environment(&EnvironmentPreparationRequest::new(
            "/runtime/python",
            "/approved/environment",
        ))
        .await
        .expect("the five-member provider remains usable");
    assert!(result.is_none());
    Ok(())
}

#[tokio::test]
async fn an_advertised_provider_returns_its_environment_commands() -> TinyBusResult<()> {
    let (_peer, provider) = routed_with_capability(true).await?;
    let route = Route {
        language: Language::nodejs(),
        bus_name: names::providers::PYTHON.into(),
        provider: Arc::new(provider),
    };
    let request = EnvironmentPreparationRequest::new("/runtime/python", "/approved/environment")
        .with_installer_upgrade()
        .with_package_group(
            PackageGroup::new(["torch"]).with_source("https://packages.invalid/cpu"),
        );
    let plan = route
        .prepare_environment(&request)
        .await
        .expect("the advertised operation reaches its provider")
        .expect("the capability was advertised");
    assert_eq!(plan.executable, "/approved/environment/bin/python");
    assert_eq!(plan.steps[0].executable, "/runtime/python");
    assert_eq!(plan.steps[0].args, ["torch"]);
    assert_eq!(plan.steps[0].env[0].1, "https://packages.invalid/cpu");
    assert_eq!(plan.steps[0].timeout_ms, Some(1_800_000));
    Ok(())
}

#[tokio::test]
async fn a_name_nobody_owns_is_reported_as_the_provider_being_unavailable() -> TinyBusResult<()> {
    // Not as a transport error: a host reading this wants to know Python is not
    // serving, not how tinybus renders a name it never chose.
    let provider = unrouted().await?;

    for outcome in [
        provider.describe().await.err(),
        provider
            .detect_system(&RuntimeSettings::new("3.12"))
            .await
            .err(),
        provider
            .select_distribution(&RuntimeSettings::new("3.12"))
            .await
            .err(),
        provider
            .layout("/cache", &RuntimeSettings::new("3.12"))
            .await
            .err(),
        provider.harness().await.err(),
    ] {
        let error = outcome.expect("an unrouted call cannot succeed");
        assert!(
            matches!(error, Error::ProviderUnavailable { .. }),
            "got {error:?}"
        );
        assert!(
            error.is_retryable(),
            "the module may simply not be loaded yet"
        );
    }
    Ok(())
}

#[tokio::test]
async fn a_bus_provider_describes_where_it_routes_rather_than_its_connection() -> TinyBusResult<()>
{
    let (_peer, provider) = routed().await?;
    let rendered = format!("{provider:?}");
    assert!(rendered.contains("nodejs"), "got {rendered}");
    assert!(rendered.contains(BUS_NAME), "got {rendered}");
    Ok(())
}

#[tokio::test]
async fn a_provider_is_addressed_at_the_path_derived_from_its_bus_name() -> TinyBusResult<()> {
    // Serving at any other path would make the module unroutable, because
    // `tinybus_module!` derives the manifest path from the bus name.
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());

    let peer = Connection::connect(bus.connect().await?).await?;
    // Deliberately the *wrong* path: the shared interface's own, which an
    // earlier revision of this crate used.
    peer.serve_at(
        names::object_path_for(names::PROVIDER_INTERFACE)
            .as_str()
            .try_into()?,
        Served {
            preparation_capability: false,
        },
    )
    .await?;
    peer.request_name(BUS_NAME).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let provider = BusProvider::new(client, Language::nodejs(), BUS_NAME);

    let error = provider
        .describe()
        .await
        .expect_err("a provider at the wrong path is not reachable");
    assert!(
        matches!(error, Error::ProviderUnavailable { .. }),
        "got {error:?}"
    );
    Ok(())
}

#[tokio::test]
async fn a_registry_route_is_usable_through_a_trait_object() -> TinyBusResult<()> {
    let (_peer, provider) = routed().await?;
    let boxed: Arc<dyn Provider> = Arc::new(provider);
    assert_eq!(
        boxed.describe().await.expect("routes").display_name,
        "Served"
    );
    Ok(())
}
