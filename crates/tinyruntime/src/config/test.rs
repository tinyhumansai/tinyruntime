//! Unit tests for the module configuration.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinyruntime_bus::{Language, names};

use super::{ModuleConfig, ProviderRoute};

#[test]
fn the_default_routes_the_first_party_providers() {
    let config = ModuleConfig::default();
    assert_eq!(config.providers.len(), 2);
    assert_eq!(config.providers[0].language, Language::nodejs());
    assert_eq!(config.providers[0].bus_name, names::providers::NODEJS);
    assert_eq!(config.providers[1].language, Language::python());
}

#[test]
fn an_empty_configuration_object_still_yields_the_defaults() {
    // A host that supplies `{}` means "the usual", not "route nothing".
    let config: ModuleConfig = serde_json::from_value(serde_json::json!({})).unwrap();
    assert_eq!(config, ModuleConfig::default());
}

#[test]
fn a_host_can_route_a_language_somewhere_else() {
    let config: ModuleConfig = serde_json::from_value(serde_json::json!({
        "providers": [{ "language": "nodejs", "bus_name": "ai.example.MyNode" }]
    }))
    .unwrap();
    assert_eq!(
        config.providers,
        vec![ProviderRoute::new(Language::nodejs(), "ai.example.MyNode")]
    );
}

#[test]
fn a_host_can_route_a_language_this_build_never_heard_of() {
    // The point of configuring routes rather than compiling them in: a new
    // provider module must not require a new router build.
    let config: ModuleConfig = serde_json::from_value(serde_json::json!({
        "providers": [{ "language": "ruby", "bus_name": "ai.example.Ruby" }]
    }))
    .unwrap();
    assert_eq!(config.providers[0].language, Language::new("ruby"));
}

#[test]
fn an_explicit_harness_directory_is_honoured() {
    let config = ModuleConfig {
        harness_dir: "/var/lib/tinyruntime".to_string(),
        ..ModuleConfig::default()
    };
    assert_eq!(
        config.harness_root(),
        std::path::Path::new("/var/lib/tinyruntime")
    );
}

#[test]
fn the_default_harness_directory_is_under_a_cache() {
    let root = ModuleConfig::default().harness_root();
    assert!(root.ends_with("harnesses"), "got {}", root.display());
}
