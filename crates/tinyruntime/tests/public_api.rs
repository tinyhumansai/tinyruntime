//! Integration tests for the public crate surface.
//!
//! These link against the crate as a downstream consumer would: they can only
//! use what `src/lib.rs` re-exports. Treat them as the regression suite for the
//! crate's public contract — if a change breaks a test here, it is a breaking
//! change for users.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use tinyruntime::{
    Engine, Error, ExecRequest, Language, ModuleConfig, Registry, ResolveRequest, RuntimeSettings,
    names,
};

/// An engine routing nothing, which is enough to exercise the public surface.
fn engine() -> Engine {
    Engine::new(
        Registry::new(),
        reqwest::Client::new(),
        std::env::temp_dir().join("tinyruntime-public-api"),
    )
}

#[test]
fn the_contract_is_re_exported_so_consumers_take_one_dependency() {
    // These are the same types the module serves, not copies of them.
    let request = ExecRequest::new(
        Language::nodejs(),
        RuntimeSettings::new("v22.11.0"),
        "console.log(1)",
    );
    let same: tinyruntime_bus::ExecRequest = request.clone();
    assert_eq!(same, request);
}

#[test]
fn the_bus_identity_is_available_without_naming_a_string() {
    assert_eq!(names::INTERFACE, tinyruntime::INTERFACE);
    assert_eq!(names::METHODS.len(), 4);
    assert!(names::PROVIDER_METHODS.contains(&names::provider_methods::DESCRIBE));
}

#[test]
fn a_default_configuration_routes_the_first_party_providers() {
    let config = ModuleConfig::default();
    assert_eq!(config.providers.len(), 2);
}

#[tokio::test]
async fn an_engine_routing_nothing_reports_an_unknown_language() {
    let error = engine()
        .resolve(&ResolveRequest::probe(
            Language::nodejs(),
            RuntimeSettings::new("v22.11.0"),
        ))
        .await
        .expect_err("nothing is registered");
    assert!(matches!(error, Error::UnknownLanguage(_)), "got {error:?}");
}

#[tokio::test]
async fn a_request_naming_no_language_is_refused_rather_than_defaulted() {
    let error = engine()
        .resolve(&ResolveRequest::probe(
            Language::new("  "),
            RuntimeSettings::new("1.0.0"),
        ))
        .await
        .expect_err("an empty language is refused");
    assert!(matches!(error, Error::LanguageMissing), "got {error:?}");
}

#[tokio::test]
async fn an_engine_reports_no_pools_before_anything_runs() {
    assert!(engine().pool_stats().await.is_empty());
}
