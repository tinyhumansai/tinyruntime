//! Persistent recipe ownership: scoped handles, cross-process lock and owned staging.
mod filesystem;
mod identity;
mod validate;

use cap_fs_ext::{FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use fs2::FileExt;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tinyruntime_bus::worker::{CacheAdoptionPolicy, CacheArtifact, CacheRecipe, WorkerCacheScope};
use tokio::sync::watch;
use tokio::time::Instant;

pub(super) fn recipe(recipe: &CacheRecipe) -> Result<(), &'static str> {
    validate::recipe(recipe)
}

pub(super) fn scopes(scopes: &[WorkerCacheScope]) -> Result<(), &'static str> {
    if scopes.len() > 32 {
        return Err("cache_scope_limit");
    }
    for (index, scope) in scopes.iter().enumerate() {
        validate::scope(scope)?;
        for prior in &scopes[..index] {
            if prior.id == scope.id
                || Path::new(&scope.root).starts_with(&prior.root)
                || Path::new(&prior.root).starts_with(&scope.root)
            {
                return Err("cache_scope_conflict");
            }
        }
    }
    Ok(())
}

/// Kept in Actor until native reaping and owned staging cleanup both succeed.
#[derive(Debug)]
pub(super) struct Session {
    root: Dir,
    path: PathBuf,
    lock: Option<File>,
    lock_identity: (u64, u64),
    recipe_identity: CacheArtifact,
    staging: Option<Dir>,
    staged: Vec<String>,
    pub(super) cached: bool,
    pub(super) legacy: bool,
}

impl Session {
    pub(super) async fn acquire(
        path: &Path,
        recipe: &CacheRecipe,
        stop: &mut watch::Receiver<bool>,
        deadline: Instant,
    ) -> Result<Self, &'static str> {
        let recipe_identity = identity::artifact(recipe)?;
        let root = filesystem::absolute(path, true)?;
        let options = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .follow(FollowSymlinks::No)
            .nonblock(true)
            .clone();
        let lock = root
            .open_with(".tinyruntime-cache.lock", &options)
            .map_err(|_| "cache_lock_failed")?;
        let metadata = lock.metadata().map_err(|_| "cache_lock_failed")?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err("cache_lock_unsafe");
        }
        let identity = (metadata.dev(), metadata.ino());
        let lock = lock.into_std();
        loop {
            if *stop.borrow() {
                return Err("canceled");
            }
            if Instant::now() >= deadline {
                return Err("cache_timeout");
            }
            match lock.try_lock_exclusive() {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    tokio::select! {
                        biased;
                        () = super::native::canceled(stop) => return Err("canceled"),
                        () = tokio::time::sleep_until(deadline.min(Instant::now() + Duration::from_millis(25))) => {},
                    }
                }
                Err(_) => return Err("cache_lock_failed"),
            }
        }
        let mut session = Self {
            root,
            path: path.to_owned(),
            lock: Some(lock),
            lock_identity: identity,
            recipe_identity,
            staging: None,
            staged: Vec::new(),
            cached: false,
            legacy: false,
        };
        session.check_binding()?;
        match session.readiness(recipe)? {
            Readiness::Cached => session.cached = true,
            Readiness::Legacy => session.legacy = true,
            Readiness::Rebuild => {}
        }
        Ok(session)
    }

    pub(super) fn check_binding(&self) -> Result<(), &'static str> {
        let current = filesystem::absolute(&self.path, false)?;
        if !filesystem::same_directory(&self.root, &current)? {
            return Err("cache_scope_changed");
        }
        let options = OpenOptions::new()
            .read(true)
            .follow(FollowSymlinks::No)
            .nonblock(true)
            .clone();
        let lock = self
            .root
            .open_with(".tinyruntime-cache.lock", &options)
            .map_err(|_| "cache_lock_changed")?;
        let metadata = lock.metadata().map_err(|_| "cache_lock_changed")?;
        if metadata.nlink() != 1 || (metadata.dev(), metadata.ino()) != self.lock_identity {
            return Err("cache_lock_changed");
        }
        Ok(())
    }

    fn readiness(&self, recipe: &CacheRecipe) -> Result<Readiness, &'static str> {
        if !filesystem::matches(&self.root, &recipe.marker)? {
            return Ok(Readiness::Rebuild);
        }
        for entry in &recipe.required {
            if !filesystem::required(&self.root, entry)? {
                return Ok(Readiness::Rebuild);
            }
        }
        for artifact in &recipe.artifacts {
            if !filesystem::matches(&self.root, artifact)? {
                return Ok(Readiness::Rebuild);
            }
        }
        if filesystem::regular_file_exists(&self.root, &self.recipe_identity.path)? {
            return if filesystem::matches(&self.root, &self.recipe_identity)? {
                Ok(Readiness::Cached)
            } else {
                Ok(Readiness::Rebuild)
            };
        }
        Ok(match recipe.adoption {
            CacheAdoptionPolicy::Strict => Readiness::Rebuild,
            CacheAdoptionPolicy::AdoptVerifiedLegacy => Readiness::Legacy,
        })
    }

    pub(super) fn install(&mut self, recipe: &CacheRecipe) -> Result<(), &'static str> {
        self.check_binding()?;
        filesystem::invalidate(&self.root, &recipe.marker.path)?;
        let name = format!(".tinyruntime-stage-{}", uuid::Uuid::new_v4());
        self.root
            .create_dir(&name)
            .map_err(|_| "cache_stage_failed")?;
        self.staging = Some(
            self.root
                .open_dir_nofollow(&name)
                .map_err(|_| "cache_stage_failed")?,
        );
        let stage = self.staging.as_ref().ok_or("cache_stage_failed")?;
        for (index, artifact) in recipe.artifacts.iter().enumerate() {
            let name = format!("artifact-{index}");
            self.staged.push(name.clone());
            filesystem::stage(stage, &name, &artifact.bytes)?;
            self.check_binding()?;
            filesystem::promote(stage, &name, &self.root, &artifact.path)?;
        }
        self.staged.push("identity".into());
        filesystem::stage(stage, "identity", &self.recipe_identity.bytes)?;
        self.staged.push("marker".into());
        filesystem::stage(stage, "marker", &recipe.marker.bytes)
    }

    pub(super) fn adopt_legacy(&mut self, recipe: &CacheRecipe) -> Result<(), &'static str> {
        self.check_binding()?;
        if !self.legacy || self.readiness(recipe)? != Readiness::Legacy {
            return Err("cache_legacy_changed");
        }
        let name = format!(".tinyruntime-stage-{}", uuid::Uuid::new_v4());
        self.root
            .create_dir(&name)
            .map_err(|_| "cache_stage_failed")?;
        self.staging = Some(
            self.root
                .open_dir_nofollow(&name)
                .map_err(|_| "cache_stage_failed")?,
        );
        self.staged.push("identity".into());
        filesystem::stage(
            self.staging.as_ref().ok_or("cache_stage_failed")?,
            "identity",
            &self.recipe_identity.bytes,
        )?;
        self.check_binding()?;
        filesystem::promote(
            self.staging.as_ref().ok_or("cache_stage_failed")?,
            "identity",
            &self.root,
            &self.recipe_identity.path,
        )?;
        self.legacy = false;
        self.cached = true;
        Ok(())
    }

    pub(super) fn publish(&self, recipe: &CacheRecipe) -> Result<(), &'static str> {
        self.check_binding()?;
        for entry in &recipe.required {
            if !filesystem::required(&self.root, entry)? {
                return Err("cache_required_missing");
            }
        }
        for artifact in &recipe.artifacts {
            if !filesystem::matches(&self.root, artifact)? {
                return Err("cache_artifact_changed");
            }
        }
        let stage = self.staging.as_ref().ok_or("cache_stage_failed")?;
        filesystem::promote(stage, "identity", &self.root, &self.recipe_identity.path)?;
        filesystem::promote(stage, "marker", &self.root, &recipe.marker.path)
    }

    pub(super) fn cleanup(&mut self) -> Result<(), &'static str> {
        if let Some(stage) = &self.staging {
            for name in &self.staged {
                match stage.remove_file_or_symlink(name) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err("cache_stage_cleanup_failed"),
                }
            }
            // Remove only this opened directory, never recursively delete user data.
            stage
                .try_clone()
                .map_err(|_| "cache_stage_cleanup_failed")?
                .remove_open_dir()
                .map_err(|_| "cache_stage_cleanup_failed")?;
        }
        self.staging = None;
        self.staged.clear();
        self.lock = None;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Readiness {
    Cached,
    Legacy,
    Rebuild,
}

use cap_fs_ext::DirExt;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
