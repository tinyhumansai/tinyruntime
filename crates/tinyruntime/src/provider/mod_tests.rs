//! Unit tests for provider routing.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use tinyruntime_bus::{CONTRACT_VERSION, Language, ProviderDescriptor};

use super::stub::{DownProvider, StubProvider};
use super::{Provider, Registry, Route, verify_contract};
use crate::error::Error;

fn registry_with_node() -> Registry {
    crate::testing::evaluate_log_fields();
    let mut registry = Registry::new();
    registry.register(
        &Language::nodejs(),
        "ai.tinyhumans.runtime.nodejs.Provider",
        Arc::new(StubProvider::new(Language::nodejs())),
    );
    registry
}

#[test]
fn an_empty_registry_routes_nothing() {
    let registry = Registry::new();
    assert!(registry.is_empty());
    assert!(matches!(
        registry.provider(&Language::nodejs()),
        Err(Error::UnknownLanguage(_))
    ));
}

#[test]
fn a_registered_language_routes_to_its_provider() {
    let registry = registry_with_node();
    assert_eq!(registry.len(), 1);
    assert!(registry.provider(&Language::nodejs()).is_ok());
}

#[test]
fn lookup_normalises_the_language_the_caller_spelled() {
    // A host that writes "NodeJS" in its configuration must reach the same
    // provider as one that writes "nodejs".
    let registry = registry_with_node();
    assert!(registry.provider(&Language::new("NodeJS")).is_ok());
}

#[test]
fn a_request_naming_no_language_is_refused_rather_than_defaulted() {
    let registry = registry_with_node();
    assert!(matches!(
        registry.provider(&Language::new("  ")),
        Err(Error::LanguageMissing)
    ));
}

#[test]
fn re_registering_a_language_replaces_its_route_without_duplicating_it() {
    let mut registry = registry_with_node();
    registry.register(
        &Language::nodejs(),
        "ai.tinyhumans.runtime.nodejs.Override",
        Arc::new(StubProvider::new(Language::nodejs())),
    );
    assert_eq!(registry.len(), 1, "the language was listed twice");
    assert_eq!(registry.languages(), vec![Language::nodejs()]);
}

#[test]
fn registration_order_is_the_order_languages_are_reported_in() {
    let mut registry = registry_with_node();
    registry.register(
        &Language::python(),
        "ai.tinyhumans.runtime.python.Provider",
        Arc::new(StubProvider::new(Language::python())),
    );
    assert_eq!(
        registry.languages(),
        vec![Language::nodejs(), Language::python()]
    );
}

#[tokio::test]
async fn a_serving_provider_is_reported_available() {
    let statuses = registry_with_node().statuses().await;
    assert_eq!(statuses.len(), 1);
    assert!(statuses[0].available);
    assert_eq!(statuses[0].display_name.as_deref(), Some("Stub"));
    assert!(statuses[0].detail.is_none());
}

#[tokio::test]
async fn a_provider_that_is_down_is_a_listed_row_with_a_reason() {
    // One unloaded provider must not turn the whole listing into an error and
    // hide every language that *is* working.
    let mut registry = registry_with_node();
    registry.register(
        &Language::python(),
        "ai.tinyhumans.runtime.python.Provider",
        Arc::new(DownProvider(Language::python())),
    );

    let statuses = registry.statuses().await;
    assert_eq!(statuses.len(), 2);
    assert!(statuses[0].available, "the working language was hidden");
    assert!(!statuses[1].available);
    assert!(
        statuses[1]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("not loaded"),
        "got {:?}",
        statuses[1].detail
    );
}

#[tokio::test]
async fn a_provider_on_a_future_contract_is_reported_unavailable() {
    let mut registry = Registry::new();
    let (major, minor) = CONTRACT_VERSION;
    registry.register(
        &Language::python(),
        "ai.tinyhumans.runtime.python.Provider",
        Arc::new(StubProvider::new(Language::python()).with_contract((major + 1, minor))),
    );

    let statuses = registry.statuses().await;
    assert!(!statuses[0].available);
    assert!(
        statuses[0]
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("contract"),
        "got {:?}",
        statuses[0].detail
    );
}

#[test]
fn the_contract_gate_accepts_this_build_and_refuses_another_major() {
    let language = Language::nodejs();
    let current = ProviderDescriptor::new(language.clone(), "Stub", "1.0.0");
    assert!(verify_contract(&language, &current).is_ok());

    let (major, minor) = CONTRACT_VERSION;
    let mut future = current;
    future.contract_version = (major + 1, minor);
    assert!(matches!(
        verify_contract(&language, &future),
        Err(Error::ProviderContract { .. })
    ));
}

#[tokio::test]
async fn a_provider_that_is_down_fails_every_member_the_same_way() {
    // Whichever question the router happens to ask first, the answer is the
    // same: this language is unavailable, and it is worth retrying because the
    // module may simply not be loaded yet.
    let provider = DownProvider(Language::python());
    let settings = tinyruntime_bus::RuntimeSettings::new("3.12");

    let failures = [
        provider.describe().await.err(),
        provider.detect_system(&settings).await.err(),
        provider.select_distribution(&settings).await.err(),
        provider.layout("/cache", &settings).await.err(),
        provider.harness().await.err(),
    ];
    for failure in failures {
        let error = failure.expect("a down provider cannot answer");
        assert!(
            matches!(error, Error::ProviderUnavailable { .. }),
            "got {error:?}"
        );
        assert!(error.is_retryable());
    }
}

#[tokio::test]
async fn a_stub_without_a_distribution_or_harness_says_so() {
    // Guards the test double itself: a stub that silently returned something
    // would let an install test pass without installing.
    let stub = StubProvider::new(Language::nodejs());
    let settings = tinyruntime_bus::RuntimeSettings::new("1.0.0");

    assert!(stub.select_distribution(&settings).await.is_err());
    assert!(stub.harness().await.is_err());
    assert!(
        stub.layout("/anywhere", &settings)
            .await
            .expect("layout answers")
            .is_none()
    );
}

#[test]
fn a_route_describes_where_it_points_rather_than_its_provider() {
    let route = Route {
        language: Language::nodejs(),
        bus_name: "ai.example.Provider".to_string(),
        provider: Arc::new(StubProvider::new(Language::nodejs())),
    };
    let rendered = format!("{route:?}");
    assert!(rendered.contains("nodejs"), "got {rendered}");
    assert!(rendered.contains("ai.example.Provider"), "got {rendered}");
}
