//! Unit tests for the provisioning vocabulary.

use super::{ArchiveFormat, Distribution, ProviderDescriptor, RuntimeLayout};
use crate::{CONTRACT_VERSION, Language};

#[test]
fn archive_extensions_match_the_names_channels_publish() {
    assert_eq!(ArchiveFormat::TarGz.extension(), "tar.gz");
    assert_eq!(ArchiveFormat::TarXz.extension(), "tar.xz");
    assert_eq!(ArchiveFormat::Zip.extension(), "zip");
}

#[test]
fn install_dir_name_drops_the_archive_extension() {
    let dist = Distribution::new(
        "22.11.0",
        "node-v22.11.0-linux-x64.tar.xz",
        "https://nodejs.org/dist/v22.11.0/node-v22.11.0-linux-x64.tar.xz",
        ArchiveFormat::TarXz,
    );
    assert_eq!(dist.install_dir_name, "node-v22.11.0-linux-x64");
}

#[test]
fn install_dir_name_survives_an_unexpected_archive_name() {
    // A channel that names an archive without the extension the format implies
    // must not produce an empty install directory.
    let dist = Distribution::new("1.0.0", "toolchain", "https://example.invalid/t", ArchiveFormat::Zip);
    assert_eq!(dist.install_dir_name, "toolchain");
}

#[test]
fn a_distribution_without_a_digest_says_so() {
    let dist = Distribution::new("1.0.0", "t.zip", "https://example.invalid/t.zip", ArchiveFormat::Zip);
    assert!(dist.expected_sha256.is_none());
    assert_eq!(
        dist.with_sha256("ab".repeat(32)).expected_sha256.as_deref(),
        Some("ab".repeat(32).as_str())
    );
}

#[test]
fn builders_override_the_derived_directory_and_add_headers() {
    let dist = Distribution::new("3.12.4", "cpython.tar.gz", "https://example.invalid/c", ArchiveFormat::TarGz)
        .with_install_dir_name("cpython-3.12.4")
        .with_header("Accept", "application/vnd.github+json");
    assert_eq!(dist.install_dir_name, "cpython-3.12.4");
    assert_eq!(
        dist.headers,
        vec![("Accept".to_string(), "application/vnd.github+json".to_string())]
    );
}

#[test]
fn a_layout_addresses_executables_by_logical_name() {
    let layout = RuntimeLayout::new("22.11.0", "/cache/node/bin")
        .with_executable("node", "/cache/node/bin/node")
        .with_executable("npm", "/cache/node/bin/npm");
    assert_eq!(layout.executable("node"), Some("/cache/node/bin/node"));
    assert_eq!(layout.executable("npx"), None, "an absent tool is absent, not guessed");
}

#[test]
fn a_descriptor_reports_the_contract_it_was_built_against() {
    let descriptor = ProviderDescriptor::new(Language::nodejs(), "Node.js", "v22.11.0")
        .with_executable("node")
        .with_executable("npm");
    assert_eq!(descriptor.contract_version, CONTRACT_VERSION);
    assert_eq!(descriptor.executables, vec!["node".to_string(), "npm".to_string()]);
}

#[test]
fn the_distribution_wire_form_is_pinned() {
    let dist = Distribution::new("1.2.3", "t.tar.gz", "https://example.invalid/t.tar.gz", ArchiveFormat::TarGz)
        .with_sha256("cd".repeat(32));
    let value = serde_json::to_value(&dist).expect("distribution serialises");
    assert_eq!(value["format"], serde_json::json!("tar_gz"));
    assert_eq!(value["install_dir_name"], serde_json::json!("t"));
    assert_eq!(value["expected_sha256"], serde_json::json!("cd".repeat(32)));

    let decoded: Distribution = serde_json::from_value(value).expect("distribution round-trips");
    assert_eq!(decoded, dist);
}

#[test]
fn an_absent_toolchain_is_an_ordinary_answer() {
    assert_eq!(super::LayoutResponse::missing().layout, None);
    assert_eq!(
        super::LayoutResponse::found(layout_for_test()).layout,
        Some(layout_for_test())
    );
}

#[test]
fn a_layout_request_names_the_directory_it_asks_about() {
    assert_eq!(
        super::LayoutRequest::new("/cache/node-v22.11.0").install_dir,
        "/cache/node-v22.11.0"
    );
}

fn layout_for_test() -> RuntimeLayout {
    RuntimeLayout::new("22.11.0", "/cache/node-v22.11.0/bin")
        .with_executable("node", "/cache/node-v22.11.0/bin/node")
}
