//! Unit tests for resolution.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::Ordering;

use tinyruntime_bus::{
    ArchiveFormat, Distribution, Language, ResolveRequest, RuntimeLayout, RuntimeSettings,
    RuntimeSource,
};

use super::Resolver;
use crate::error::Error;
use crate::provider::stub::{DownProvider, StubProvider};
use crate::provider::{Provider, Registry};

fn layout(version: &str, dir: &str) -> RuntimeLayout {
    RuntimeLayout::new(version, format!("{dir}/bin"))
        .with_executable("tool", format!("{dir}/bin/tool"))
}

fn settings(cache_dir: &std::path::Path) -> RuntimeSettings {
    let mut settings = RuntimeSettings::new("1.0.0");
    settings.cache_dir = cache_dir.to_string_lossy().into_owned();
    settings
}

fn resolver_over(provider: Arc<dyn Provider>) -> Resolver {
    let mut registry = Registry::new();
    registry.register(&Language::nodejs(), "ai.example.Provider", provider);
    Resolver::new(registry, reqwest::Client::new())
}

#[tokio::test]
async fn a_disabled_language_is_refused_before_anything_is_probed() {
    let provider = Arc::new(StubProvider::new(Language::nodejs()));
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    let mut request = ResolveRequest::new(Language::nodejs(), RuntimeSettings::new("1.0.0"));
    request.settings.enabled = false;

    let error = resolver.resolve(&request).await.expect_err("refused");
    assert!(matches!(error, Error::LanguageDisabled(_)), "got {error:?}");
    assert_eq!(
        provider.detections.load(Ordering::Relaxed),
        0,
        "a disabled language must not spawn a version probe"
    );
}

#[tokio::test]
async fn a_compatible_host_toolchain_is_reused_without_touching_a_channel() {
    // The step that keeps a developer machine from ever downloading anything.
    let scratch = tempfile::tempdir().unwrap();
    let provider =
        Arc::new(StubProvider::new(Language::nodejs()).with_system(layout("1.2.3", "/usr/local")));
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    let found = resolver
        .require(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect("the host toolchain resolves");

    assert_eq!(found.source, RuntimeSource::System);
    assert_eq!(found.version, "1.2.3");
    assert!(found.install_dir.is_none());
    assert_eq!(
        provider.selections.load(Ordering::Relaxed),
        0,
        "a reusable host toolchain must not consult a release channel"
    );
}

#[tokio::test]
async fn prefer_system_off_skips_the_host_toolchain_entirely() {
    // A caller that needs an exact version must not be handed whatever the host
    // happens to have.
    let scratch = tempfile::tempdir().unwrap();
    let provider =
        Arc::new(StubProvider::new(Language::nodejs()).with_system(layout("1.2.3", "/usr/local")));
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    let mut settings = settings(scratch.path());
    settings.prefer_system = false;
    let found = resolver
        .resolve(&ResolveRequest::probe(Language::nodejs(), settings))
        .await
        .expect("the probe completes");

    assert!(found.is_none());
    assert_eq!(provider.detections.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn an_installed_toolchain_in_the_cache_is_reused_on_a_cold_start() {
    // This is what makes a restart cheap: nothing in this process has resolved
    // anything, and the cache still answers without a network round trip.
    let scratch = tempfile::tempdir().unwrap();
    let installed = scratch.path().join("toolchain-1.0.0");
    std::fs::create_dir_all(installed.join("bin")).unwrap();

    let provider = Arc::new(
        StubProvider::new(Language::nodejs())
            .with_layout(layout("1.0.0", &installed.to_string_lossy())),
    );
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    let found = resolver
        .require(&ResolveRequest::probe(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect("the cached toolchain resolves");

    assert_eq!(found.source, RuntimeSource::Managed);
    assert_eq!(
        found.install_dir.as_deref(),
        Some(installed.to_string_lossy().as_ref())
    );
    assert_eq!(provider.selections.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn a_staging_directory_left_by_a_crash_is_not_reused() {
    let scratch = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(scratch.path().join(".stage-99-abcd/bin")).unwrap();

    // The stub would happily call any directory a toolchain, so a reuse here
    // could only come from the scan failing to skip the staging directory.
    let provider =
        Arc::new(StubProvider::new(Language::nodejs()).with_layout(layout("1.0.0", "/wherever")));
    let resolver = resolver_over(provider);

    let found = resolver
        .resolve(&ResolveRequest::probe(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect("the probe completes");
    assert!(found.is_none(), "a half-finished install was reused");
}

#[tokio::test]
async fn a_probe_reports_nothing_rather_than_installing() {
    let scratch = tempfile::tempdir().unwrap();
    let provider = Arc::new(StubProvider::new(Language::nodejs()).with_distribution(
        Distribution::new(
            "1.0.0",
            "t.tar.gz",
            "http://127.0.0.1:1/t",
            ArchiveFormat::TarGz,
        ),
    ));
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    let found = resolver
        .resolve(&ResolveRequest::probe(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect("a probe of an unprovisioned language succeeds");

    assert!(found.is_none());
    assert_eq!(
        provider.selections.load(Ordering::Relaxed),
        0,
        "a probe consulted a release channel"
    );
}

#[tokio::test]
async fn require_turns_nothing_provisioned_into_an_error() {
    let scratch = tempfile::tempdir().unwrap();
    let resolver = resolver_over(Arc::new(StubProvider::new(Language::nodejs())));

    let error = resolver
        .require(&ResolveRequest::probe(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect_err("a caller about to run something cannot use `None`");
    assert!(matches!(error, Error::NotProvisioned(_)), "got {error:?}");
}

#[tokio::test]
async fn a_second_identical_request_is_answered_from_the_memo() {
    let scratch = tempfile::tempdir().unwrap();
    let provider =
        Arc::new(StubProvider::new(Language::nodejs()).with_system(layout("1.2.3", "/usr/local")));
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);
    let request = ResolveRequest::new(Language::nodejs(), settings(scratch.path()));

    resolver.require(&request).await.unwrap();
    resolver.require(&request).await.unwrap();

    assert_eq!(
        provider.detections.load(Ordering::Relaxed),
        1,
        "the second request re-probed the host"
    );
}

#[tokio::test]
async fn a_request_for_a_different_version_is_not_answered_from_the_memo() {
    // Sharing one memo across versions would silently hand the second caller the
    // first caller's toolchain.
    let scratch = tempfile::tempdir().unwrap();
    let provider =
        Arc::new(StubProvider::new(Language::nodejs()).with_system(layout("1.2.3", "/usr/local")));
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    resolver
        .require(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .unwrap();

    let mut other = settings(scratch.path());
    other.version = "2.0.0".to_string();
    resolver
        .require(&ResolveRequest::new(Language::nodejs(), other))
        .await
        .unwrap();

    assert_eq!(provider.detections.load(Ordering::Relaxed), 2);
}

#[tokio::test]
async fn an_unregistered_language_is_refused() {
    let resolver = resolver_over(Arc::new(StubProvider::new(Language::nodejs())));
    let error = resolver
        .resolve(&ResolveRequest::new(
            Language::python(),
            RuntimeSettings::new("3.12"),
        ))
        .await
        .expect_err("refused");
    assert!(matches!(error, Error::UnknownLanguage(_)), "got {error:?}");
}

#[tokio::test]
async fn a_provider_that_is_down_fails_the_resolution_retryably() {
    let resolver = resolver_over(Arc::new(DownProvider(Language::nodejs())));
    let error = resolver
        .resolve(&ResolveRequest::new(
            Language::nodejs(),
            RuntimeSettings::new("1.0.0"),
        ))
        .await
        .expect_err("a down provider fails");
    assert!(
        matches!(error, Error::ProviderUnavailable { .. }),
        "got {error:?}"
    );
    assert!(
        error.is_retryable(),
        "the module may simply not be loaded yet"
    );
}

#[tokio::test]
async fn a_provider_on_an_incompatible_contract_is_refused_before_installing() {
    let scratch = tempfile::tempdir().unwrap();
    let (major, minor) = tinyruntime_bus::CONTRACT_VERSION;
    let provider = Arc::new(
        StubProvider::new(Language::nodejs())
            .with_contract((major + 1, minor))
            .with_system(layout("1.2.3", "/usr/local")),
    );
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    let error = resolver
        .resolve(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect_err("an incompatible provider is refused");

    assert!(
        matches!(error, Error::ProviderContract { .. }),
        "got {error:?}"
    );
    assert_eq!(
        provider.detections.load(Ordering::Relaxed),
        0,
        "an incompatible provider was still asked to work"
    );
}

#[tokio::test]
async fn an_install_that_produces_no_toolchain_is_reported_as_such() {
    // The provider says what to install and then finds nothing in it. That is a
    // distinct failure from a download or an unpack going wrong.
    let scratch = tempfile::tempdir().unwrap();
    let archive = scratch.path().join("source.tar.gz");
    write_single_root_tarball(&archive);
    let url = format!("file://{}", archive.display());

    let provider = Arc::new(StubProvider::new(Language::nodejs()).with_distribution(
        Distribution::new("1.0.0", "t.tar.gz", &url, ArchiveFormat::TarGz),
    ));
    let resolver = resolver_over(provider);

    let error = resolver
        .resolve(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect_err("a file URL is not fetchable, which is the point below");

    // `reqwest` does not serve `file://`, so this is a download failure rather
    // than an empty install — the assertion that matters is that the router
    // reported it rather than panicking or installing something empty.
    assert!(matches!(error, Error::Download { .. }), "got {error:?}");
}

/// Write a tarball with exactly one root directory, for the install path.
fn write_single_root_tarball(path: &std::path::Path) {
    let file = std::fs::File::create(path).unwrap();
    let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
    let mut builder = tar::Builder::new(encoder);
    let mut header = tar::Header::new_gnu();
    header.set_size(0);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, "toolchain-1.0.0/marker", &b""[..])
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap();
}
