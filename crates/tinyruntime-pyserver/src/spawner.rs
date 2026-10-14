//! The seam through which a host decides how the Python worker is launched.
//!
//! This crate never decides isolation. A host that has an execution policy (a
//! `TinyBox` plan, a jail, a container) installs a [`Spawner`] on the
//! [`ServerLaunch`](crate::ServerLaunch); the worker is then described as a
//! [`SpawnSpec`], handed to the spawner, and only the spec it returns is run.
//! The shape mirrors `runtime_python_server::Spawner`; this crate does not depend on
//! the router, so the two are twins rather than one type.
//!
//! The spawner **fails closed**: when it refuses, the spawn fails and nothing is
//! started. There is no fallback to a direct launch, because a fallback is exactly
//! the unconfined run the host asked to prevent.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::process::Command;

/// A process to start, as data: what a [`Spawner`] reads and rewrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    /// The program to execute.
    pub program: PathBuf,
    /// Arguments after the program.
    pub args: Vec<String>,
    /// Environment variables to set on the child.
    pub env: Vec<(String, String)>,
    /// Whether the child's inherited environment is cleared before `env` is set.
    pub clear_env: bool,
}

impl SpawnSpec {
    /// Build the command this spec describes. Stdio and process flags are the
    /// caller's to set.
    #[must_use]
    pub fn to_command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.args);
        if self.clear_env {
            command.env_clear();
        }
        for (name, value) in &self.env {
            command.env(name, value);
        }
        command
    }
}

/// A spawner declined to start the process.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("the spawner refused to start the process: {0}")]
pub struct SpawnRefused(pub String);

/// Decides how a process is confined before it is started.
pub trait Spawner: Send + Sync + fmt::Debug {
    /// A stable identity for this policy. It takes part in the pool fingerprint,
    /// so changing the confinement rebuilds the warm workers instead of serving
    /// from workers started under the old one.
    fn id(&self) -> &str;

    /// Rewrite `spec` so that it runs under the host's confinement, for example
    /// by wrapping the program in a sandbox launcher.
    ///
    /// # Errors
    ///
    /// Returns [`SpawnRefused`] when the process may not be started at all. The
    /// caller does not retry directly.
    fn confine(&self, spec: SpawnSpec) -> Result<SpawnSpec, SpawnRefused>;
}

/// A shareable spawner.
pub type SharedSpawner = Arc<dyn Spawner>;

/// Run `spec` through `spawner`, or unchanged when the host installed none.
///
/// # Errors
///
/// Returns [`SpawnRefused`] when the installed spawner refuses.
pub fn confine(
    spawner: Option<&SharedSpawner>,
    spec: SpawnSpec,
) -> Result<SpawnSpec, SpawnRefused> {
    match spawner {
        Some(spawner) => {
            tracing::debug!(
                spawner = spawner.id(),
                "[tinyruntime_pyserver] confining a spawn through the host spawner"
            );
            spawner.confine(spec)
        }
        None => Ok(spec),
    }
}

#[cfg(test)]
#[path = "spawner_tests.rs"]
mod tests;
