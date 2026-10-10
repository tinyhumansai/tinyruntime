//! Tests for the module adapter and for routing over a real bus.
//!
//! The interesting one is [`resolve_routes_to_a_provider_module`]: a second peer
//! on the same bus serves the provider interface, and the router reaches it for
//! an answer it could not have produced itself. That is the whole design working
//! end to end — two modules, one contract, no language knowledge in the router.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(clippy::unused_async)]

use tinybus::broker::Broker;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Interface, Result as TinyBusResult};

use tinyruntime_bus::worker::WorkerCommand;
use tinyruntime_bus::{
    EnvironmentPreparationPlan, EnvironmentPreparationRequest, ExecRequest, ExecResponse, Language,
    LanguagesResponse, LayoutRequest, LayoutResponse, PackageGroup, PrepareEnvironmentRequest,
    ProviderCapability, ProviderDescriptor, ResolveRequest, ResolveResponse, RuntimeLayout,
    RuntimeSettings, RuntimeSource, WorkerHarness, names,
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
        std::future::ready(Ok(LayoutResponse::found(host_toolchain()))).await
    }

    async fn layout(&self, request: LayoutRequest) -> TinyBusResult<LayoutResponse> {
        // Echo the directory back through the version, so a test can prove the
        // request crossed rather than being answered from a constant.
        std::future::ready(Ok(LayoutResponse::found(RuntimeLayout::new(
            request.install_dir,
            "/installed/bin",
        ))))
        .await
    }

    async fn harness(&self) -> TinyBusResult<WorkerHarness> {
        std::future::ready(Ok(worker_harness())).await
    }
}

/// The logical executable the fake harness runs under.
const TOOL: &str = "tool";

/// This test binary, presented as an installed toolchain.
///
/// Running a job needs a real executable on the other end, and re-executing this
/// binary as a worker keeps that from depending on Node being installed.
fn host_toolchain() -> RuntimeLayout {
    let binary = std::env::current_exe().expect("a test binary has a path");
    let bin_dir = binary
        .parent()
        .expect("the binary is in a directory")
        .to_string_lossy()
        .into_owned();
    RuntimeLayout::new("1.2.3", bin_dir)
        .with_executable(TOOL, binary.to_string_lossy().into_owned())
}

/// A harness whose flags make the test binary serve the worker protocol.
fn worker_harness() -> WorkerHarness {
    let launch = crate::pool::fake_worker::launch(Language::nodejs());
    let mut harness = WorkerHarness::new("worker-harness", "unused by this worker", TOOL)
        .with_env(crate::pool::fake_worker::WORKER_MARKER, "1");
    harness.args_before_script = launch.args.clone();
    harness
}

/// A second provider, so the two cannot be confused for one another.
struct OtherFakeProvider;

/// A provider whose optional environment callback is observable at the router boundary.
struct RecipeFakeProvider;

#[tinybus::interface(name = "ai.tinyhumans.runtime.Provider")]
impl RecipeFakeProvider {
    async fn describe(&self) -> TinyBusResult<ProviderDescriptor> {
        Ok(
            ProviderDescriptor::new(Language::nodejs(), "Recipe Node.js", "1.0.0")
                .with_capability(ProviderCapability::PrepareEnvironment),
        )
    }

    async fn detect_system(&self, _settings: RuntimeSettings) -> TinyBusResult<LayoutResponse> {
        Ok(LayoutResponse::found(host_toolchain()))
    }

    async fn layout(&self, request: LayoutRequest) -> TinyBusResult<LayoutResponse> {
        Ok(LayoutResponse::found(RuntimeLayout::new(
            request.install_dir,
            "/installed/bin",
        )))
    }

    async fn harness(&self) -> TinyBusResult<WorkerHarness> {
        Ok(worker_harness())
    }

    async fn prepare_environment(
        &self,
        request: tinyruntime_bus::EnvironmentPreparationRequest,
    ) -> TinyBusResult<EnvironmentPreparationPlan> {
        let packages = request
            .package_groups
            .iter()
            .flat_map(|group| group.packages.iter().cloned())
            .collect::<Vec<_>>();
        Ok(EnvironmentPreparationPlan {
            executable: format!("{}/bin/python", request.environment_root),
            steps: vec![WorkerCommand {
                executable: request.runtime_executable,
                args: packages,
                env: Vec::new(),
                timeout_ms: Some(120_000),
            }],
        })
    }
}

/// An advertised provider with an incompatible contract, which must be refused.
struct IncompatibleRecipeProvider;

#[tinybus::interface(name = "ai.tinyhumans.runtime.Provider")]
impl IncompatibleRecipeProvider {
    async fn describe(&self) -> TinyBusResult<ProviderDescriptor> {
        let mut descriptor = ProviderDescriptor::new(Language::nodejs(), "Future Node.js", "1.0.0")
            .with_capability(ProviderCapability::PrepareEnvironment);
        descriptor.contract_version = (2, 0);
        Ok(descriptor)
    }

    async fn detect_system(&self, _settings: RuntimeSettings) -> TinyBusResult<LayoutResponse> {
        Ok(LayoutResponse::missing())
    }

    async fn layout(&self, _request: LayoutRequest) -> TinyBusResult<LayoutResponse> {
        Ok(LayoutResponse::missing())
    }

    async fn harness(&self) -> TinyBusResult<WorkerHarness> {
        Ok(worker_harness())
    }
}

#[tinybus::interface(name = "ai.tinyhumans.runtime.Provider")]
impl OtherFakeProvider {
    async fn describe(&self) -> TinyBusResult<ProviderDescriptor> {
        std::future::ready(Ok(ProviderDescriptor::new(
            Language::python(),
            "Fake Python",
            "3.12",
        )))
        .await
    }

    async fn detect_system(&self, _settings: RuntimeSettings) -> TinyBusResult<LayoutResponse> {
        std::future::ready(Ok(LayoutResponse::missing())).await
    }

    async fn layout(&self, _request: LayoutRequest) -> TinyBusResult<LayoutResponse> {
        std::future::ready(Ok(LayoutResponse::missing())).await
    }

    async fn harness(&self) -> TinyBusResult<WorkerHarness> {
        std::future::ready(Ok(WorkerHarness::new(
            "pool_worker.py",
            "# harness",
            "python",
        )))
        .await
    }
}

/// The bus name the fake provider claims in these tests.
const FAKE_BUS_NAME: &str = names::providers::NODEJS;

/// Start a broker and return a bus every peer in a test connects through.
fn bus() -> MemoryBus {
    crate::testing::evaluate_log_fields();
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    bus
}

/// A configuration routing only Node.js, at the fake provider's bus name.
fn config_routing_node(harness_dir: &std::path::Path) -> ModuleConfig {
    ModuleConfig {
        providers: vec![ProviderRoute::new(Language::nodejs(), FAKE_BUS_NAME)],
        harness_dir: harness_dir.to_string_lossy().into_owned(),
        worker_cache_scopes: Vec::new(),
    }
}

#[test]
fn declared_methods_match_the_dispatch_table() {
    let service = RuntimeService {
        workers: crate::worker::WorkerManager::new(std::path::PathBuf::from("unused-test-root")),
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
        workers: crate::worker::WorkerManager::new(std::path::PathBuf::from("unused-test-root")),
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
        .serve_at(
            names::object_path_for(FAKE_BUS_NAME).as_str().try_into()?,
            FakeProvider,
        )
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
async fn host_recipe_request_is_negotiated_through_the_router_for_an_old_provider()
-> TinyBusResult<()> {
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();

    let provider = Connection::connect(bus.connect().await?).await?;
    provider
        .serve_at(
            names::object_path_for(FAKE_BUS_NAME).as_str().try_into()?,
            FakeProvider,
        )
        .await?;
    provider.request_name(FAKE_BUS_NAME).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: Option<EnvironmentPreparationPlan> = proxy
        .call(
            names::methods::PREPARE_ENVIRONMENT,
            (PrepareEnvironmentRequest::new(
                Language::nodejs(),
                EnvironmentPreparationRequest::new("/usr/bin/node", "/approved/env"),
            ),),
        )
        .await?;

    assert!(reply.is_none(), "an old five-member provider has no recipe");
    Ok(())
}

#[tokio::test]
async fn host_recipe_request_round_trips_through_the_router_to_an_advertising_provider()
-> TinyBusResult<()> {
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();

    let provider = Connection::connect(bus.connect().await?).await?;
    provider
        .serve_at(
            names::object_path_for(FAKE_BUS_NAME).as_str().try_into()?,
            RecipeFakeProvider,
        )
        .await?;
    provider.request_name(FAKE_BUS_NAME).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let request = PrepareEnvironmentRequest::new(
        Language::nodejs(),
        EnvironmentPreparationRequest::new("/runtime/python", "/approved/env")
            .with_package_group(PackageGroup::new(["numpy", "torch"])),
    );
    let reply: Option<EnvironmentPreparationPlan> = proxy
        .call(names::methods::PREPARE_ENVIRONMENT, (request,))
        .await?;
    let plan = reply.expect("advertised provider returns its native recipe");

    assert_eq!(plan.executable, "/approved/env/bin/python");
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].executable, "/runtime/python");
    assert_eq!(plan.steps[0].args, ["numpy", "torch"]);
    assert_eq!(plan.steps[0].timeout_ms, Some(120_000));
    Ok(())
}

#[tokio::test]
async fn router_refuses_an_incompatible_provider_recipe_without_fallback() -> TinyBusResult<()> {
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();

    let provider = Connection::connect(bus.connect().await?).await?;
    provider
        .serve_at(
            names::object_path_for(FAKE_BUS_NAME).as_str().try_into()?,
            IncompatibleRecipeProvider,
        )
        .await?;
    provider.request_name(FAKE_BUS_NAME).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(module, config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let error = proxy
        .call::<Option<EnvironmentPreparationPlan>>(
            names::methods::PREPARE_ENVIRONMENT,
            (PrepareEnvironmentRequest::new(
                Language::nodejs(),
                EnvironmentPreparationRequest::new("/runtime/python", "/approved/env"),
            ),),
        )
        .await
        .expect_err("an incompatible provider cannot supply a recipe");

    assert!(error.to_string().contains("contract 2.0"), "{error}");
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
        .serve_at(
            names::object_path_for(FAKE_BUS_NAME).as_str().try_into()?,
            FakeProvider,
        )
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
    assert!(
        runtime.executable(TOOL).is_some(),
        "the provider's toolchain did not cross the bus"
    );
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
    let error = proxy
        .call::<ResolveResponse>(
            names::methods::RESOLVE,
            (ResolveRequest::probe(
                Language::python(),
                RuntimeSettings::new("3.12"),
            ),),
        )
        .await
        .expect_err("an unrouted language cannot resolve");
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

#[tokio::test]
async fn each_provider_is_addressed_at_its_own_object_path() -> TinyBusResult<()> {
    // The bug this rules out: addressing every provider at one shared path.
    // `tinybus_module!` derives a module's manifest path from its bus name, so a
    // provider serving at a shared path would ship a manifest that disagreed
    // with the object it exports — and a router addressing one would reach the
    // wrong object, or none. Two providers here, each at its own derived path.
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();

    let node = Connection::connect(bus.connect().await?).await?;
    node.serve_at(
        names::object_path_for(names::providers::NODEJS)
            .as_str()
            .try_into()?,
        FakeProvider,
    )
    .await?;
    node.request_name(names::providers::NODEJS).await?;

    let python = Connection::connect(bus.connect().await?).await?;
    python
        .serve_at(
            names::object_path_for(names::providers::PYTHON)
                .as_str()
                .try_into()?,
            OtherFakeProvider,
        )
        .await?;
    python.request_name(names::providers::PYTHON).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(
        module.clone(),
        ModuleConfig {
            providers: vec![
                ProviderRoute::new(Language::nodejs(), names::providers::NODEJS),
                ProviderRoute::new(Language::python(), names::providers::PYTHON),
            ],
            harness_dir: scratch.path().to_string_lossy().into_owned(),
            worker_cache_scopes: Vec::new(),
        },
    )
    .await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let reply: LanguagesResponse = proxy.call(names::methods::LANGUAGES, ()).await?;

    assert_eq!(reply.languages.len(), 2);
    assert!(
        reply.languages[0].available,
        "{:?}",
        reply.languages[0].detail
    );
    assert_eq!(
        reply.languages[0].display_name.as_deref(),
        Some("Fake Node.js")
    );
    assert!(
        reply.languages[1].available,
        "{:?}",
        reply.languages[1].detail
    );
    assert_eq!(
        reply.languages[1].display_name.as_deref(),
        Some("Fake Python"),
        "the router reached the wrong provider's object"
    );
    Ok(())
}

#[tokio::test]
async fn a_host_runs_code_through_the_router_over_the_bus() -> TinyBusResult<()> {
    // The whole system in one call: a host asks the router to run something, the
    // router asks a provider module what a toolchain and a worker are, and a
    // real child process runs the job.
    let bus = bus();
    let scratch = tempfile::tempdir().expect("scratch directory");

    let provider = Connection::connect(bus.connect().await?).await?;
    provider
        .serve_at(
            names::object_path_for(FAKE_BUS_NAME).as_str().try_into()?,
            FakeProvider,
        )
        .await?;
    provider.request_name(FAKE_BUS_NAME).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(module.clone(), config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    let mut settings = RuntimeSettings::new("1.0.0");
    settings.cache_dir = scratch.path().to_string_lossy().into_owned();
    let request = ExecRequest::new(
        Language::nodejs(),
        settings,
        crate::pool::fake_worker::Directive::Echo("over-the-bus").code(),
    );

    let reply: ExecResponse = proxy.call(names::methods::EXECUTE, (request,)).await?;
    assert_eq!(reply.stdout, "over-the-bus");
    assert!(reply.success());
    assert_eq!(reply.runtime_version, "1.2.3");

    // And the pool it built is now reportable.
    let pools: tinyruntime_bus::PoolStatsResponse =
        proxy.call(names::methods::POOL_STATS, ()).await?;
    assert_eq!(pools.pools.len(), 1);
    assert_eq!(pools.pools[0].jobs_total, 1);
    Ok(())
}

#[tokio::test]
async fn a_job_for_a_language_with_no_provider_fails_with_a_readable_reason() -> TinyBusResult<()> {
    let bus = bus();
    let scratch = tempfile::tempdir().expect("scratch directory");
    let module = Connection::connect(bus.connect().await?).await?;
    setup(module.clone(), config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    let error = proxy
        .call::<ExecResponse>(
            names::methods::EXECUTE,
            (ExecRequest::new(
                Language::python(),
                RuntimeSettings::new("3.12"),
                "print(1)",
            ),),
        )
        .await
        .expect_err("an unrouted language cannot run anything");
    assert!(error.to_string().contains("python"), "got `{error}`");
    Ok(())
}

#[tokio::test]
async fn a_cached_install_is_reported_through_the_routers_resolve() -> TinyBusResult<()> {
    // Exercises the provider's `Layout` member, which the reuse scan is what
    // actually calls.
    let bus = bus();
    let scratch = tempfile::tempdir().expect("scratch directory");
    std::fs::create_dir_all(scratch.path().join("cache/toolchain-1.0.0"))
        .expect("a cached install");

    let provider = Connection::connect(bus.connect().await?).await?;
    provider
        .serve_at(
            names::object_path_for(FAKE_BUS_NAME).as_str().try_into()?,
            FakeProvider,
        )
        .await?;
    provider.request_name(FAKE_BUS_NAME).await?;

    let module = Connection::connect(bus.connect().await?).await?;
    setup(module.clone(), config_routing_node(scratch.path())).await?;

    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;

    let mut settings = RuntimeSettings::new("1.0.0");
    settings.prefer_system = false;
    settings.cache_dir = scratch.path().join("cache").to_string_lossy().into_owned();
    let reply: ResolveResponse = proxy
        .call(
            names::methods::RESOLVE,
            (ResolveRequest::probe(Language::nodejs(), settings),),
        )
        .await?;

    let runtime = reply.runtime.expect("the cached install is reported");
    assert!(
        runtime.version.ends_with("toolchain-1.0.0"),
        "the install directory did not reach the provider: {}",
        runtime.version
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn generic_persistent_worker_members_dispatch_real_jsonl_and_cleanup() -> TinyBusResult<()> {
    use tinyruntime_bus::worker::{
        WorkerCommand, WorkerHandle, WorkerOutcome, WorkerOutcomeKind, WorkerPlan, WorkerPrepare,
        WorkerRequest, WorkerStatus,
    };
    let bus = bus();
    let scratch = tempfile::tempdir().unwrap();
    let module = Connection::connect(bus.connect().await?).await?;
    let mut config = config_routing_node(scratch.path());
    config.worker_cache_scopes = vec![tinyruntime_bus::worker::WorkerCacheScope {
        id: "cache".into(),
        root: scratch
            .path()
            .canonicalize()
            .unwrap()
            .join("recipe-cache")
            .to_string_lossy()
            .into_owned(),
    }];
    setup(module, config).await?;
    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let handle: WorkerHandle = proxy.call(names::methods::WORKER_RESERVE, ()).await?;
    let source = r#"printf '%s\n' '{"ready":true,"protocol":1,"backends":["alpha"]}'
while IFS= read -r line; do
 id=${line#*\"id\":\"}; id=${id%%\"*}
 printf '{"id":"%s","ok":true,"result":{"answer":42}}\n' "$id"
done
"#;
    let plan = WorkerPlan {
        source: source.into(),
        command: WorkerCommand {
            executable: "/bin/sh".into(),
            args: Vec::new(),
            env: Vec::new(),
            timeout_ms: None,
        },
        preparation: Vec::new(),
        backends: vec!["alpha".into()],
        startup_timeout_ms: 30_000,
        request_timeout_ms: 60_000,
        idle_backend: None,
        idle_timeout_ms: 0,
    };
    let prepared: WorkerOutcome = proxy
        .call(
            names::methods::WORKER_PREPARE,
            (WorkerPrepare {
                handle: handle.clone(),
                plan: plan.clone(),
            },),
        )
        .await?;
    assert_eq!(prepared.kind, WorkerOutcomeKind::Complete);
    let started: WorkerOutcome = proxy
        .call(names::methods::WORKER_START, (handle.clone(),))
        .await?;
    assert_eq!(started.kind, WorkerOutcomeKind::Complete);
    let status: WorkerStatus = proxy
        .call(names::methods::WORKER_STATUS, (handle.clone(),))
        .await?;
    assert!(status.server.running);
    assert!(status.server.backends[0].ready);
    let response: WorkerOutcome = proxy
        .call(
            names::methods::WORKER_REQUEST,
            (WorkerRequest {
                handle: handle.clone(),
                operation: 1,
                method: "alpha.run".into(),
                params: serde_json::json!({"input":"fixture"}),
            },),
        )
        .await?;
    assert_eq!(
        response.response.unwrap().result,
        Some(serde_json::json!({"answer":42}))
    );
    let stopped: WorkerOutcome = proxy.call(names::methods::WORKER_STOP, (handle,)).await?;
    assert_eq!(stopped.kind, WorkerOutcomeKind::Complete);
    let shutdown: WorkerOutcome = proxy.call(names::methods::WORKER_SHUTDOWN, ()).await?;
    assert_eq!(shutdown.kind, WorkerOutcomeKind::Complete);
    assert!(
        proxy
            .call::<WorkerHandle>(names::methods::WORKER_RESERVE, ())
            .await
            .is_err()
    );
    Ok(())
}

#[cfg(unix)]
#[tokio::test]
async fn cached_worker_member_publishes_exact_marker_bytes() -> TinyBusResult<()> {
    use tinyruntime_bus::worker::{
        WorkerCommand, WorkerHandle, WorkerOutcome, WorkerOutcomeKind, WorkerPlan,
        WorkerPrepareCached,
    };
    let scratch = tempfile::tempdir().unwrap();
    let bus = bus();
    let module = Connection::connect(bus.connect().await?).await?;
    let mut config = config_routing_node(scratch.path());
    config.worker_cache_scopes = vec![tinyruntime_bus::worker::WorkerCacheScope {
        id: "cache".into(),
        root: scratch
            .path()
            .canonicalize()
            .unwrap()
            .join("recipe-cache")
            .to_string_lossy()
            .into_owned(),
    }];
    setup(module, config).await?;
    let client = Connection::connect(bus.connect().await?).await?;
    let proxy = client.proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)?;
    let handle: WorkerHandle = proxy.call(names::methods::WORKER_RESERVE, ()).await?;
    let prepared: WorkerOutcome = proxy
        .call(
            names::methods::WORKER_PREPARE_CACHED,
            (WorkerPrepareCached {
                handle: handle.clone(),
                plan: WorkerPlan {
                    source: "exit 0".into(),
                    command: WorkerCommand {
                        executable: "/bin/sh".into(),
                        args: Vec::new(),
                        env: Vec::new(),
                        timeout_ms: None,
                    },
                    preparation: Vec::new(),
                    backends: Vec::new(),
                    startup_timeout_ms: 1000,
                    request_timeout_ms: 1000,
                    idle_backend: None,
                    idle_timeout_ms: 0,
                },
                recipe: tinyruntime_bus::worker::CacheRecipe {
                    scope: "cache".into(),
                    artifacts: Vec::new(),
                    steps: Vec::new(),
                    required: Vec::new(),
                    marker: tinyruntime_bus::worker::CacheArtifact {
                        path: "ready".into(),
                        bytes: b"exact".to_vec(),
                    },
                    adoption: tinyruntime_bus::worker::CacheAdoptionPolicy::Strict,
                    timeout_ms: 1000,
                },
            },),
        )
        .await?;
    assert_eq!(prepared.kind, WorkerOutcomeKind::Complete);
    assert_eq!(
        std::fs::read(scratch.path().join("recipe-cache/ready")).unwrap(),
        b"exact"
    );
    let stopped: WorkerOutcome = proxy.call(names::methods::WORKER_STOP, (handle,)).await?;
    assert_eq!(stopped.kind, WorkerOutcomeKind::Complete);
    Ok(())
}
