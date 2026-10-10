//! Host-approved cache scopes and bounded declarative provisioning vocabulary.
use super::{WorkerHandle, WorkerPlan};
use serde::{Deserialize, Serialize};

/// A named absolute cache location approved only by module-load configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerCacheScope {
    /// Stable name that recipes reference instead of supplying arbitrary roots.
    pub id: String,
    /// Explicit absolute cache location; no implicit scope is admitted.
    pub root: String,
}

/// One bounded router-owned artifact or readiness marker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheArtifact {
    /// Portable relative path under the approved cache scope.
    pub path: String,
    /// Exact bytes; no language-specific encoding or interpretation.
    pub bytes: Vec<u8>,
}

/// The existence/type proof a recipe requires before publishing readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheEntryKind {
    /// Regular file, without following a final symlink.
    File,
    /// Real directory, without following a final symlink.
    Directory,
    /// Any existing entry; symlink targets are not inspected or read.
    Entry,
}

/// One required path under the approved cache scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Portable relative path; intermediate links are refused.
    pub path: String,
    /// Required entry kind.
    pub kind: CacheEntryKind,
}

/// Policy for adopting a complete cache created before recipe identities existed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheAdoptionPolicy {
    /// Require a matching recipe identity; legacy entries are prepared again.
    #[default]
    Strict,
    /// Adopt only an identity-free cache whose marker, artifacts and entries match exactly.
    AdoptVerifiedLegacy,
}

/// An opaque provisioning recipe supplied by a trusted host/provider owner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheRecipe {
    /// Name of the host-approved cache scope.
    pub scope: String,
    /// Explicit router-owned files installed before preparation commands.
    pub artifacts: Vec<CacheArtifact>,
    /// Explicit native preparation commands; final cache paths stay unchanged.
    pub steps: Vec<super::WorkerCommand>,
    /// Entries required before readiness publication and on cache reuse.
    pub required: Vec<CacheEntry>,
    /// Exact readiness bytes published only after successful native cleanup.
    pub marker: CacheArtifact,
    /// Whether a complete identity-free legacy cache may be adopted.
    #[serde(default)]
    pub adoption: CacheAdoptionPolicy,
    /// Whole lock/provisioning deadline in milliseconds.
    pub timeout_ms: u64,
}

/// Prepare a known worker reservation through a persistent scoped cache recipe.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerPrepareCached {
    /// Caller-known reservation that owns cancellation and native cleanup.
    pub handle: WorkerHandle,
    /// Unchanged worker launch/transient preparation plan.
    pub plan: WorkerPlan,
    /// Immutable persistent provisioning recipe.
    pub recipe: CacheRecipe,
}
