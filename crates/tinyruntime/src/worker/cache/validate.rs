//! Portable scope/path checks and bounds before any cache access.
use std::collections::HashSet;
use std::path::{Component, Path};
use tinyruntime_bus::worker::{CacheRecipe, WorkerCacheScope};

pub(super) const MAX_FILE: usize = 64 * 1024;

fn forbidden(value: &str) -> bool {
    matches!(
        value.to_ascii_lowercase().as_str(),
        ".ssh" | ".gnupg" | ".aws" | ".azure" | ".kube" | ".config-gcloud"
    )
}

pub(super) fn id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

pub(super) fn scope(scope: &WorkerCacheScope) -> Result<(), &'static str> {
    let path = Path::new(&scope.root);
    if !id(&scope.id) || scope.root.len() > 4096 || !path.is_absolute() || scope.root.contains('\0')
    {
        return Err("cache_scope_invalid");
    }
    let mut count = 0;
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or("cache_scope_invalid")?;
                if forbidden(value) {
                    return Err("cache_scope_forbidden");
                }
                count += 1;
            }
            Component::Prefix(_) | Component::RootDir => {}
            _ => return Err("cache_scope_invalid"),
        }
    }
    // Never grant a whole filesystem/system root or an entire home directory.
    if count < 2 || dirs::home_dir().is_some_and(|home| path == home) {
        return Err("cache_scope_forbidden");
    }
    #[cfg(unix)]
    if [
        "/etc", "/proc", "/sys", "/dev", "/bin", "/sbin", "/usr", "/boot",
    ]
    .iter()
    .any(|root| path.starts_with(root))
    {
        return Err("cache_scope_forbidden");
    }
    Ok(())
}

pub(super) fn relative(path: &str) -> Result<(), &'static str> {
    safe_relative(path)?;
    if path
        .split('/')
        .any(|part| part.starts_with(".tinyruntime-"))
    {
        return Err("cache_path_invalid");
    }
    Ok(())
}

pub(super) fn safe_relative(path: &str) -> Result<(), &'static str> {
    if path.is_empty()
        || path.len() > 512
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
    {
        return Err("cache_path_invalid");
    }
    for part in path.split('/') {
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if part.is_empty()
            || matches!(part, "." | "..")
            || forbidden(part)
            || part.ends_with(['.', ' '])
            || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit())
        {
            return Err("cache_path_invalid");
        }
    }
    Ok(())
}

pub(super) fn recipe(recipe: &CacheRecipe) -> Result<(), &'static str> {
    if !id(&recipe.scope)
        || recipe.artifacts.len() > 16
        || recipe.required.len() > 32
        || recipe.steps.len() > 16
        || recipe.timeout_ms == 0
        || recipe.timeout_ms > 30 * 60 * 1000
    {
        return Err("cache_recipe_limit");
    }
    let mut paths = HashSet::new();
    let mut bytes = 0_usize;
    for artifact in recipe
        .artifacts
        .iter()
        .chain(std::iter::once(&recipe.marker))
    {
        relative(&artifact.path)?;
        if artifact.bytes.len() > MAX_FILE || !paths.insert(&artifact.path) {
            return Err("cache_artifact_limit");
        }
        bytes = bytes.saturating_add(artifact.bytes.len());
    }
    if bytes > 1024 * 1024 {
        return Err("cache_artifact_limit");
    }
    for required in &recipe.required {
        relative(&required.path)?;
    }
    for step in &recipe.steps {
        super::super::validate::command(step)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "validate_tests.rs"]
mod tests;
