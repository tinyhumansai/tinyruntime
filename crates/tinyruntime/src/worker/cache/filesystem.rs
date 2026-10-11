//! Directory capabilities keep cache writes and cleanup within opened scopes.
use super::validate::MAX_FILE;
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use tinyruntime_bus::worker::{CacheArtifact, CacheEntry, CacheEntryKind};

fn failed(_: std::io::Error) -> &'static str {
    "cache_io_failed"
}

/// Open each absolute component without following links, creating only when allowed.
pub(super) fn absolute(path: &Path, create: bool) -> Result<Dir, &'static str> {
    let mut anchor = PathBuf::new();
    let mut names = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => anchor.push(component.as_os_str()),
            Component::Normal(name) => names.push(name),
            _ => return Err("cache_scope_invalid"),
        }
    }
    let mut dir = Dir::open_ambient_dir(anchor, cap_std::ambient_authority()).map_err(failed)?;
    for name in names {
        dir = descend(&dir, Path::new(name), create)?;
    }
    Ok(dir)
}

fn descend(dir: &Dir, name: &Path, create: bool) -> Result<Dir, &'static str> {
    match dir.open_dir_nofollow(name) {
        Ok(child) => Ok(child),
        Err(error) if create && error.kind() == std::io::ErrorKind::NotFound => {
            match dir.create_dir(name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(failed(error)),
            }
            dir.open_dir_nofollow(name).map_err(failed)
        }
        Err(error) => Err(failed(error)),
    }
}

pub(super) fn parent(dir: &Dir, path: &str, create: bool) -> Result<(Dir, String), &'static str> {
    super::validate::safe_relative(path)?;
    let mut parts = path.split('/').peekable();
    let mut dir = dir.try_clone().map_err(failed)?;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            return Ok((dir, part.to_owned()));
        }
        dir = descend(&dir, Path::new(part), create)?;
    }
    Err("cache_path_invalid")
}

pub(super) fn same_directory(first: &Dir, second: &Dir) -> Result<bool, &'static str> {
    let first = first.dir_metadata().map_err(failed)?;
    let second = second.dir_metadata().map_err(failed)?;
    Ok(first.dev() == second.dev() && first.ino() == second.ino())
}

pub(super) fn matches(dir: &Dir, artifact: &CacheArtifact) -> Result<bool, &'static str> {
    let Ok((parent, name)) = parent(dir, &artifact.path, false) else {
        return Ok(false);
    };
    let options = OpenOptions::new()
        .read(true)
        .follow(FollowSymlinks::No)
        .nonblock(true)
        .clone();
    let mut file = match parent.open_with(&name, &options) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(failed(error)),
    };
    let metadata = file.metadata().map_err(failed)?;
    if !metadata.is_file() || metadata.nlink() != 1 {
        return Err("cache_entry_unsafe");
    }
    if metadata.len() > MAX_FILE as u64 || metadata.len() != artifact.bytes.len() as u64 {
        return Ok(false);
    }
    let mut bytes = Vec::with_capacity(artifact.bytes.len());
    Read::by_ref(&mut file)
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(failed)?;
    Ok(bytes == artifact.bytes)
}

pub(super) fn regular_file_exists(dir: &Dir, path: &str) -> Result<bool, &'static str> {
    let Ok((parent, name)) = parent(dir, path, false) else {
        return Ok(false);
    };
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.is_file() && metadata.nlink() == 1 => Ok(true),
        Ok(_) => Err("cache_entry_unsafe"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err("cache_io_failed"),
    }
}

pub(super) fn required(dir: &Dir, entry: &CacheEntry) -> Result<bool, &'static str> {
    let Ok((parent, name)) = parent(dir, &entry.path, false) else {
        return Ok(false);
    };
    let metadata = match parent.symlink_metadata(name) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(failed(error)),
    };
    Ok(match entry.kind {
        CacheEntryKind::File => metadata.is_file() && metadata.nlink() == 1,
        CacheEntryKind::Directory => metadata.is_dir(),
        CacheEntryKind::Entry => metadata.is_file() || metadata.is_dir() || metadata.is_symlink(),
    })
}

pub(super) fn stage(dir: &Dir, name: &str, bytes: &[u8]) -> Result<(), &'static str> {
    let options = OpenOptions::new()
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No)
        .nonblock(true)
        .clone();
    let mut file = dir.open_with(name, &options).map_err(failed)?;
    file.write_all(bytes).map_err(failed)?;
    file.sync_all().map_err(failed)
}

pub(super) fn promote(
    stage: &Dir,
    name: &str,
    root: &Dir,
    destination: &str,
) -> Result<(), &'static str> {
    let (parent, target) = parent(root, destination, true)?;
    if let Ok(metadata) = parent.symlink_metadata(&target)
        && (!metadata.is_file() || metadata.nlink() != 1)
    {
        return Err("cache_entry_unsafe");
    }
    stage.rename(name, &parent, target).map_err(failed)
}

pub(super) fn invalidate(root: &Dir, path: &str) -> Result<(), &'static str> {
    let Ok((parent, name)) = parent(root, path, false) else {
        return Ok(());
    };
    match parent.symlink_metadata(&name) {
        Ok(metadata) if metadata.is_file() && metadata.nlink() == 1 => {
            parent.remove_file(name).map_err(failed)
        }
        Ok(_) => Err("cache_entry_unsafe"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(failed(error)),
    }
}

#[cfg(test)]
#[path = "filesystem_tests.rs"]
mod tests;
