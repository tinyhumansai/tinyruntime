//! Unit tests for the bus identity constants.

use super::{
    INTERFACE, METHODS, OBJECT_PATH, PROVIDER_INTERFACE, PROVIDER_METHODS, methods,
    object_path_for, provider_methods, providers,
};

#[test]
fn the_routers_object_path_is_its_bus_name_in_path_form() {
    assert_eq!(OBJECT_PATH, object_path_for(INTERFACE));
}

#[test]
fn each_providers_object_path_is_derived_from_its_own_bus_name() {
    // Not from the shared interface. `tinybus_module!` builds a module's
    // manifest path from its bus name, so a provider that served at a shared
    // path would ship a manifest disagreeing with the object it exports.
    assert_eq!(
        providers::NODEJS_OBJECT_PATH,
        object_path_for(providers::NODEJS)
    );
    assert_eq!(
        providers::PYTHON_OBJECT_PATH,
        object_path_for(providers::PYTHON)
    );
    assert_ne!(
        providers::NODEJS_OBJECT_PATH,
        providers::PYTHON_OBJECT_PATH,
        "two providers cannot share one object path"
    );
    assert_ne!(
        providers::NODEJS_OBJECT_PATH,
        object_path_for(PROVIDER_INTERFACE),
        "a provider must not serve at the shared interface's path"
    );
}

#[test]
fn every_router_member_is_listed_once() {
    assert_eq!(
        METHODS,
        [
            methods::LANGUAGES,
            methods::RESOLVE,
            methods::EXECUTE,
            methods::POOL_STATS
        ]
    );
    let mut sorted = METHODS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), METHODS.len(), "a member is listed twice");
}

#[test]
fn every_provider_member_is_listed_once() {
    assert_eq!(
        PROVIDER_METHODS,
        [
            provider_methods::DESCRIBE,
            provider_methods::DETECT_SYSTEM,
            provider_methods::SELECT_DISTRIBUTION,
            provider_methods::LAYOUT,
            provider_methods::HARNESS
        ]
    );
    let mut sorted = PROVIDER_METHODS.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), PROVIDER_METHODS.len());
}

#[test]
fn providers_claim_distinct_bus_names_under_the_shared_interface() {
    assert_ne!(providers::NODEJS, providers::PYTHON);
    for name in [providers::NODEJS, providers::PYTHON] {
        assert!(
            name.starts_with("ai.tinyhumans.runtime."),
            "{name} is outside this contract's namespace"
        );
        assert_ne!(
            name, PROVIDER_INTERFACE,
            "a provider must not claim the shared interface as its bus name"
        );
    }
}
