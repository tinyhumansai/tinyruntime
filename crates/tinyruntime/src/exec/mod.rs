//! Running code: resolve, provision if needed, then hand the job to a warm
//! worker.
//!
//! This is the surface almost every host actually uses. Resolution and pooling
//! are worth exposing separately — a host that wants to pre-provision at boot,
//! or to render pool counters, needs them — but the common request is "run this,
//! and deal with whatever getting an interpreter takes", and that should be one
//! call rather than a protocol a host has to implement.

use std::path::{Path, PathBuf};
use std::time::Duration;

use reqwest::Client;

use tinyruntime_bus::{
    ExecRequest, ExecResponse, Language, PoolStats, ResolveRequest, ResolvedRuntime,
};

use crate::error::{Error, Result};
use crate::pool::{Launch, Pools, env};
use crate::provider::Registry;
use crate::resolve::Resolver;

/// The router's whole capability, in one object.
///
/// Holds the routing table, the resolver's memo, and the live pools. One per
/// module: the pools in particular are process-wide state, and two engines would
/// each keep their own warm workers and double the memory the pool exists to cap.
#[derive(Debug)]
pub struct Engine {
    resolver: Resolver,
    pools: Pools,
    /// Where worker harnesses are written.
    harness_root: PathBuf,
}

impl Engine {
    /// Build an engine over `registry`, writing harnesses under `harness_root`.
    #[must_use]
    pub fn new(registry: Registry, client: Client, harness_root: PathBuf) -> Self {
        Self {
            resolver: Resolver::new(registry, client),
            pools: Pools::new(),
            harness_root,
        }
    }

    /// The routing table this engine routes over.
    #[must_use]
    pub fn registry(&self) -> &Registry {
        self.resolver.registry()
    }

    /// Resolve a runtime without running anything.
    ///
    /// # Errors
    ///
    /// As [`Resolver::resolve`].
    pub async fn resolve(&self, request: &ResolveRequest) -> Result<Option<ResolvedRuntime>> {
        self.resolver.resolve(request).await
    }

    /// Every live pool's counters.
    pub async fn pool_stats(&self) -> Vec<PoolStats> {
        self.pools.stats().await
    }

    /// Resolve a runtime and run `request` on it.
    ///
    /// # Errors
    ///
    /// Returns whatever resolution failed with, [`Error::PoolSaturated`] when
    /// the pool is at capacity, and the dispatch variants when the job could not
    /// be run.
    pub async fn execute(&self, request: &ExecRequest) -> Result<ExecResponse> {
        let resolve = ResolveRequest::new(request.language.clone(), request.settings.clone());
        let runtime = self.resolver.require(&resolve).await?;

        let launch = self.launch_for(&runtime, &request.language).await?;
        let pool = self
            .pools
            .ensure(launch, request.pool, runtime.version.clone())
            .await;

        pool.run(
            request.code.clone(),
            request.cwd.clone(),
            request.timeout_ms.map(Duration::from_millis),
        )
        .await
    }

    /// Build the worker launch for a resolved runtime.
    ///
    /// The harness is fetched from the provider and written out on every launch
    /// build. That sounds wasteful and is not: [`Pools::ensure`] reuses a live
    /// pool for an unchanged fingerprint, so this runs once per toolchain rather
    /// than once per job.
    async fn launch_for(&self, runtime: &ResolvedRuntime, language: &Language) -> Result<Launch> {
        let provider = self.registry().provider(language)?;
        let harness = provider.harness().await?;

        let binary = runtime
            .executable(&harness.executable)
            .ok_or_else(|| Error::EmptyInstall(language.clone()))?;

        let script = env::materialise(&self.harness_root.join(language.as_str()), &harness).await?;
        let env = env::build(Path::new(&runtime.bin_dir), &harness.env);

        Ok(Launch {
            language: language.clone(),
            binary: PathBuf::from(binary),
            args: harness.command_args(&script.to_string_lossy()),
            env,
            protocol_version: harness.protocol_version,
            handshake_timeout: crate::pool::worker::DEFAULT_HANDSHAKE_TIMEOUT,
        })
    }
}

#[cfg(test)]
mod test;
