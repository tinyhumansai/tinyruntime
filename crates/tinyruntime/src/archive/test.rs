//! Unit tests for archive unpacking.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use tinyruntime_bus::{ArchiveFormat, Language};

use super::{extract, single_root};
use crate::error::Error;
use crate::testing;
use crate::testing::evaluate_log_fields;

/// Build a `.tar.gz` holding one root directory with one file in it.
fn write_tar_gz(path: &Path, root: &str) {
    let file = fs::File::create(path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    let payload = b"#!/bin/sh\n";
    header.set_size(payload.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, format!("{root}/bin/tool"), &payload[..])
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}

/// Build a `.zip` holding one root directory with one file in it.
fn write_zip(path: &Path, root: &str) {
    let file = fs::File::create(path).unwrap();
    let mut writer = zip::ZipWriter::new(file);
    writer
        .start_file(
            format!("{root}/bin/tool"),
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    writer.write_all(b"#!/bin/sh\n").unwrap();
    writer.finish().unwrap();
}

#[tokio::test]
async fn unpacks_a_tarball_and_returns_its_single_root() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.tar.gz");
    write_tar_gz(&archive, "toolchain-1.2.3");

    let root = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::TarGz,
        &Language::nodejs(),
    )
    .await
    .expect("the archive unpacks");

    assert_eq!(root.file_name().unwrap(), "toolchain-1.2.3");
    assert!(root.join("bin/tool").is_file());
}

#[cfg(unix)]
#[tokio::test]
async fn a_tarball_keeps_the_executable_bit() {
    evaluate_log_fields();
    // An interpreter that loses `+x` during extraction installs cleanly and then
    // cannot be run, which surfaces much later as a confusing spawn failure.
    use std::os::unix::fs::PermissionsExt;

    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.tar.gz");
    write_tar_gz(&archive, "toolchain-1.2.3");

    let root = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::TarGz,
        &Language::nodejs(),
    )
    .await
    .unwrap();

    let mode = fs::metadata(root.join("bin/tool"))
        .unwrap()
        .permissions()
        .mode();
    assert!(
        mode & 0o111 != 0,
        "extracted tool is not executable: {mode:o}"
    );
}

#[tokio::test]
async fn unpacks_a_zip_archive() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.zip");
    write_zip(&archive, "toolchain-1.2.3");

    let root = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::Zip,
        &Language::nodejs(),
    )
    .await
    .expect("the zip unpacks");

    assert_eq!(
        fs::read_to_string(root.join("bin/tool")).unwrap(),
        "#!/bin/sh\n"
    );
}

#[tokio::test]
async fn a_missing_archive_fails_as_an_install_error() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    let error = extract(
        &scratch.path().join("absent.tar.gz"),
        &scratch.path().join("stage"),
        ArchiveFormat::TarGz,
        &Language::python(),
    )
    .await
    .expect_err("a missing archive cannot unpack");
    assert!(matches!(error, Error::Install { .. }), "got {error:?}");
}

#[test]
fn an_archive_with_no_root_directory_is_refused() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    fs::write(scratch.path().join("loose-file"), b"x").unwrap();
    let error = single_root(scratch.path()).expect_err("a rootless archive is refused");
    assert!(error.contains("no directory"), "got `{error}`");
}

#[test]
fn an_archive_with_several_root_directories_is_refused() {
    evaluate_log_fields();
    // Promoting one of them would install something nobody chose.
    let scratch = tempfile::tempdir().unwrap();
    fs::create_dir(scratch.path().join("one")).unwrap();
    fs::create_dir(scratch.path().join("two")).unwrap();
    let error = single_root(scratch.path()).expect_err("an ambiguous archive is refused");
    assert!(error.contains("2 directories"), "got `{error}`");
}

#[test]
fn a_single_root_is_returned_ignoring_stray_files() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    fs::create_dir(scratch.path().join("toolchain-1.0.0")).unwrap();
    fs::write(scratch.path().join("LICENSE"), b"x").unwrap();
    let root: PathBuf = single_root(scratch.path()).expect("one directory is unambiguous");
    assert_eq!(root.file_name().unwrap(), "toolchain-1.0.0");
}

#[tokio::test]
async fn unpacks_an_xz_tarball() {
    evaluate_log_fields();
    // The format every Unix Node distribution ships as, and the only one with a
    // decoder this crate would otherwise never exercise.
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.tar.xz");
    fs::write(
        &archive,
        testing::single_root_archive("toolchain-1.2.3", ArchiveFormat::TarXz),
    )
    .unwrap();

    let root = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::TarXz,
        &Language::nodejs(),
    )
    .await
    .expect("the xz tarball unpacks");

    assert_eq!(root.file_name().unwrap(), "toolchain-1.2.3");
    assert_eq!(fs::read_to_string(root.join("bin/tool")).unwrap(), "#!/bin/sh\n");
}

#[cfg(unix)]
#[tokio::test]
async fn a_zip_restores_the_mode_bits_it_carries() {
    evaluate_log_fields();
    // Windows archives are zips, and an interpreter that arrives without its
    // execute bit installs cleanly and then cannot be run.
    use std::os::unix::fs::PermissionsExt;

    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.zip");
    fs::write(
        &archive,
        testing::single_root_archive("toolchain-1.2.3", ArchiveFormat::Zip),
    )
    .unwrap();

    let root = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::Zip,
        &Language::nodejs(),
    )
    .await
    .expect("the zip unpacks");

    let mode = fs::metadata(root.join("bin/tool")).unwrap().permissions().mode();
    assert!(mode & 0o111 != 0, "extracted tool is not executable: {mode:o}");
}

#[tokio::test]
async fn a_gzip_archive_that_is_not_one_fails_as_an_install_error() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.tar.gz");
    fs::write(&archive, b"not compressed at all").unwrap();

    let error = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::TarGz,
        &Language::python(),
    )
    .await
    .expect_err("garbage cannot unpack");
    assert!(matches!(error, Error::Install { .. }), "got {error:?}");
}

#[tokio::test]
async fn an_xz_archive_that_is_not_one_fails_as_an_install_error() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.tar.xz");
    fs::write(&archive, b"not compressed at all").unwrap();

    let error = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::TarXz,
        &Language::nodejs(),
    )
    .await
    .expect_err("garbage cannot unpack");
    assert!(matches!(error, Error::Install { .. }), "got {error:?}");
}

#[tokio::test]
async fn a_zip_that_is_not_one_fails_as_an_install_error() {
    evaluate_log_fields();
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("toolchain.zip");
    fs::write(&archive, b"PK not really").unwrap();

    let error = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::Zip,
        &Language::nodejs(),
    )
    .await
    .expect_err("garbage cannot unpack");
    assert!(matches!(error, Error::Install { .. }), "got {error:?}");
}

#[tokio::test]
async fn a_zip_holding_only_files_is_refused_for_having_no_root() {
    evaluate_log_fields();
    // Every toolchain archive is single-rooted. One that is not did not come
    // from where the provider said it did.
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("flat.zip");
    let mut writer = zip::ZipWriter::new(fs::File::create(&archive).unwrap());
    writer
        .start_file("loose", zip::write::SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"x").unwrap();
    writer.finish().unwrap();

    let error = extract(
        &archive,
        &scratch.path().join("stage"),
        ArchiveFormat::Zip,
        &Language::nodejs(),
    )
    .await
    .expect_err("a rootless archive is refused");
    assert!(matches!(error, Error::Install { .. }), "got {error:?}");
}
