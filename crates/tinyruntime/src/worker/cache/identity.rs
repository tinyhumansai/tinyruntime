//! Bounded recipe identities stored separately from caller-preserved marker bytes.
use sha2::{Digest, Sha256};
use std::io::{Result as IoResult, Write};
use tinyruntime_bus::worker::{CacheArtifact, CacheRecipe};

struct HashWriter(Sha256);
impl Write for HashWriter {
    fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

pub(super) fn artifact(recipe: &CacheRecipe) -> Result<CacheArtifact, &'static str> {
    let mut writer = HashWriter(Sha256::new());
    // Validated recipe bounds are enforced before this call; no serialized clone.
    serde_json::to_writer(&mut writer, recipe).map_err(|_| "cache_identity_failed")?;
    Ok(CacheArtifact {
        path: format!(
            ".tinyruntime-recipe-{}",
            hex::encode(Sha256::digest(recipe.marker.path.as_bytes()))
        ),
        bytes: hex::encode(writer.0.finalize()).into_bytes(),
    })
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
