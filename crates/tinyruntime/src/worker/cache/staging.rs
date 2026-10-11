//! Owned staging deletion retains identity across a failed Windows removal.
use cap_fs_ext::{DirExt, MetadataExt};
use cap_std::fs::Dir;

#[derive(Debug)]
pub(super) struct Staging {
    directory: Option<Dir>,
    name: String,
    identity: (u64, u64),
}

impl Staging {
    pub(super) fn open(root: &Dir, name: String) -> Result<Self, &'static str> {
        let directory = root
            .open_dir_nofollow(&name)
            .map_err(|_| "cache_stage_failed")?;
        let metadata = directory.dir_metadata().map_err(|_| "cache_stage_failed")?;
        Ok(Self {
            directory: Some(directory),
            name,
            identity: (metadata.dev(), metadata.ino()),
        })
    }

    pub(super) fn directory(&self) -> Result<&Dir, &'static str> {
        self.directory.as_ref().ok_or("cache_stage_failed")
    }

    pub(super) fn cleanup(&mut self, root: &Dir, staged: &[String]) -> Result<(), &'static str> {
        if self.directory.is_none() {
            let directory = root
                .open_dir_nofollow(&self.name)
                .map_err(|_| "cache_stage_cleanup_failed")?;
            let metadata = directory
                .dir_metadata()
                .map_err(|_| "cache_stage_cleanup_failed")?;
            if (metadata.dev(), metadata.ino()) != self.identity {
                return Err("cache_stage_cleanup_failed");
            }
            self.directory = Some(directory);
        }
        let stage = self.directory().map_err(|_| "cache_stage_cleanup_failed")?;
        for name in staged {
            match stage.remove_file_or_symlink(name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err("cache_stage_cleanup_failed"),
            }
        }
        // Windows requires closing every handle without delete-sharing before
        // removal. Keep the name and identity so a failed removal can be retried
        // against the same directory, without deleting user-injected contents.
        self.directory
            .take()
            .ok_or("cache_stage_cleanup_failed")?
            .remove_open_dir()
            .map_err(|_| "cache_stage_cleanup_failed")
    }
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
