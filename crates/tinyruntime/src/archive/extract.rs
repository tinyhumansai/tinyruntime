//! The synchronous unpacking routines, one per archive format.
//!
//! Each unpacks into `staging_dir` and nothing else — finding the promoted
//! directory and reporting failure are the caller's job, so these stay small
//! enough to read against the format they handle.

use std::fs::{self, File};
use std::io;
use std::path::Path;

/// Unpack a gzip-compressed tarball.
pub(super) fn tar_gz(archive: &Path, staging_dir: &Path) -> io::Result<()> {
    let file = File::open(archive)?;
    unpack_tar(flate2::read::GzDecoder::new(file), staging_dir)
}

/// Unpack an xz-compressed tarball.
pub(super) fn tar_xz(archive: &Path, staging_dir: &Path) -> io::Result<()> {
    let file = File::open(archive)?;
    unpack_tar(xz2::read::XzDecoder::new(file), staging_dir)
}

/// Unpack a decoded tar stream, preserving the executable bits.
///
/// `set_preserve_permissions` is the default on Unix and is restated because it
/// is load-bearing: an interpreter that loses its `+x` bit during extraction
/// installs successfully and then cannot be run.
fn unpack_tar(reader: impl io::Read, staging_dir: &Path) -> io::Result<()> {
    let mut tar = tar::Archive::new(reader);
    tar.set_preserve_permissions(true);
    tar.set_overwrite(true);
    tar.unpack(staging_dir)
}

/// Unpack a zip archive, restoring Unix mode bits where the archive carries them.
///
/// Entries whose stored path escapes the staging directory are skipped rather
/// than written: an archive that names `../../bin/node` is trying to install
/// somewhere nobody asked for.
pub(super) fn zip(archive: &Path, staging_dir: &Path) -> io::Result<()> {
    let file = File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|error| zip_io_error(&error))?;

    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|error| zip_io_error(&error))?;
        let Some(relative) = entry.enclosed_name() else {
            tracing::warn!(
                "[tinyruntime::archive] skipped a zip entry whose path escapes the staging directory"
            );
            continue;
        };
        let destination = staging_dir.join(relative);

        if entry.is_dir() {
            fs::create_dir_all(&destination)?;
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut out = File::create(&destination)?;
            io::copy(&mut entry, &mut out)?;
        }

        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&destination, fs::Permissions::from_mode(mode))?;
        }
    }

    Ok(())
}

/// Render a zip-crate failure as an IO error so every format reports the same way.
fn zip_io_error(error: &zip::result::ZipError) -> io::Error {
    io::Error::other(error.to_string())
}
