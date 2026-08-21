//! Unit tests for the resolution payloads.

use super::{ResolveRequest, ResolveResponse, ResolvedRuntime, RuntimeSource};
use crate::{Language, RuntimeLayout, RuntimeSettings};

fn layout() -> RuntimeLayout {
    RuntimeLayout::new("22.11.0", "/cache/node-v22.11.0/bin")
        .with_executable("node", "/cache/node-v22.11.0/bin/node")
}

#[test]
fn a_probe_forbids_installing() {
    let probe = ResolveRequest::probe(Language::nodejs(), RuntimeSettings::new("v22.11.0"));
    assert!(!probe.install);
    let resolve = ResolveRequest::new(Language::nodejs(), RuntimeSettings::new("v22.11.0"));
    assert!(resolve.install);
}

#[test]
fn a_resolution_carries_the_layout_it_was_built_from() {
    let runtime = ResolvedRuntime::from_layout(Language::nodejs(), RuntimeSource::Managed, layout());
    assert_eq!(runtime.version, "22.11.0");
    assert_eq!(runtime.executable("node"), Some("/cache/node-v22.11.0/bin/node"));
    assert_eq!(runtime.install_dir, None, "the directory is recorded separately");
}

#[test]
fn a_system_resolution_has_no_install_directory() {
    let runtime = ResolvedRuntime::from_layout(Language::nodejs(), RuntimeSource::System, layout());
    assert_eq!(runtime.source, RuntimeSource::System);
    assert!(runtime.install_dir.is_none());
}

#[test]
fn nothing_provisioned_is_a_successful_empty_answer() {
    assert_eq!(ResolveResponse::missing().runtime, None);
    let runtime = ResolvedRuntime::from_layout(Language::python(), RuntimeSource::System, layout());
    assert!(ResolveResponse::found(runtime).runtime.is_some());
}

#[test]
fn the_source_wire_form_is_pinned() {
    let value = serde_json::to_value(RuntimeSource::Managed).expect("source serialises");
    assert_eq!(value, serde_json::json!("managed"));
    assert_eq!(
        serde_json::to_value(RuntimeSource::System).expect("source serialises"),
        serde_json::json!("system")
    );
}
