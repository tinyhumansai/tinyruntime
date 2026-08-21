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

fn registry_for(provider: Arc<dyn Provider>) -> Registry {
    crate::testing::evaluate_log_fields();
    let mut registry = Registry::new();
    registry.register(&Language::nodejs(), "ai.example.Provider", provider);
    registry
}

fn resolver_over(provider: Arc<dyn Provider>) -> Resolver {
    Resolver::new(registry_for(provider), reqwest::Client::new())
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

// ---------------------------------------------------------------------------
// The install pipeline, against real bytes
//
// Everything above stops before the download. These run the whole thing —
// select, lock, fetch, verify, unpack, promote, re-inspect — against an archive
// served over loopback, which is the only way to cover the ordering that makes
// a concurrent install safe.
// ---------------------------------------------------------------------------

use crate::testing;

/// A provider offering `archive` at `url`, and reporting whatever is installed.
fn installing_provider(url: &str, digest: Option<&str>) -> Arc<StubProvider> {
    let mut distribution =
        Distribution::new("1.0.0", "toolchain-1.0.0.tar.gz", url, ArchiveFormat::TarGz);
    if let Some(digest) = digest {
        distribution = distribution.with_sha256(digest);
    }
    Arc::new(
        StubProvider::new(Language::nodejs())
            .with_distribution(distribution)
            // Only a directory that actually holds the unpacked tree counts, so
            // an install test cannot silently short-circuit into "already there".
            .with_layout_when_present("bin/tool", layout("1.0.0", "/installed")),
    )
}

#[tokio::test]
async fn a_managed_toolchain_is_downloaded_verified_unpacked_and_promoted() {
    let scratch = tempfile::tempdir().unwrap();
    let archive = testing::single_root_archive("toolchain-1.0.0", ArchiveFormat::TarGz);
    let digest = testing::digest(&archive);
    let (url, server) = testing::serve(archive, 1);

    let provider = installing_provider(&url, Some(&digest));
    let resolver = resolver_over(Arc::clone(&provider) as Arc<dyn Provider>);

    let installed = resolver
        .require(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect("the toolchain installs");

    assert_eq!(installed.source, RuntimeSource::Managed);
    let install_dir = scratch.path().join("toolchain-1.0.0");
    assert_eq!(
        installed.install_dir.as_deref(),
        Some(install_dir.to_string_lossy().as_ref())
    );
    assert!(
        install_dir.join("bin/tool").is_file(),
        "the archive contents were not promoted into place"
    );
    assert!(
        !scratch.path().join("toolchain-1.0.0.tar.gz").exists(),
        "the archive was left behind after a successful install"
    );
    assert_eq!(server.join().expect("the server finished"), 1);
}

#[tokio::test]
async fn a_second_resolution_reuses_the_install_rather_than_downloading_again() {
    // The server is told to serve exactly once. A second download would hang
    // and then fail, which is what makes this assertion mean something.
    let scratch = tempfile::tempdir().unwrap();
    let archive = testing::single_root_archive("toolchain-1.0.0", ArchiveFormat::TarGz);
    let digest = testing::digest(&archive);
    let (url, server) = testing::serve(archive, 1);

    let provider = installing_provider(&url, Some(&digest));
    let request = ResolveRequest::new(Language::nodejs(), settings(scratch.path()));

    // A fresh resolver each time, so the in-process memo cannot be what answers.
    Resolver::new(
        registry_for(Arc::clone(&provider) as Arc<dyn Provider>),
        reqwest::Client::new(),
    )
    .require(&request)
    .await
    .expect("the first resolution installs");

    let reused = Resolver::new(
        registry_for(Arc::clone(&provider) as Arc<dyn Provider>),
        reqwest::Client::new(),
    )
    .require(&request)
    .await
    .expect("the second resolution reuses");

    assert_eq!(reused.source, RuntimeSource::Managed);
    assert_eq!(server.join().expect("the server finished"), 1);
}

#[tokio::test]
async fn an_archive_that_fails_verification_is_not_installed() {
    // The bytes arrived intact and are the wrong bytes. Installing them would
    // give this host an interpreter nobody published.
    let scratch = tempfile::tempdir().unwrap();
    let archive = testing::single_root_archive("toolchain-1.0.0", ArchiveFormat::TarGz);
    let (url, server) = testing::serve(archive, 1);

    let provider = installing_provider(&url, Some(&"00".repeat(32)));
    let resolver = resolver_over(provider);

    let error = resolver
        .require(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect_err("a mismatched archive is refused");

    assert!(
        matches!(error, Error::DigestMismatch { .. }),
        "got {error:?}"
    );
    assert!(
        !error.is_retryable(),
        "retrying produces the same wrong bytes"
    );
    assert!(
        !scratch.path().join("toolchain-1.0.0").exists(),
        "a refused archive was installed anyway"
    );
    let _ = server.join();
}

#[tokio::test]
async fn an_install_the_provider_cannot_find_a_toolchain_in_is_reported_as_empty() {
    // The archive unpacked fine and holds nothing the provider recognises. That
    // is a distinct failure from the download or the unpacking going wrong.
    let scratch = tempfile::tempdir().unwrap();
    let archive = testing::single_root_archive("toolchain-1.0.0", ArchiveFormat::TarGz);
    let digest = testing::digest(&archive);
    let (url, server) = testing::serve(archive, 1);

    // No layout: the provider never recognises what was installed.
    let provider = Arc::new(
        StubProvider::new(Language::nodejs()).with_distribution(
            Distribution::new(
                "1.0.0",
                "toolchain-1.0.0.tar.gz",
                &url,
                ArchiveFormat::TarGz,
            )
            .with_sha256(digest),
        ),
    );
    let resolver = resolver_over(provider);

    let error = resolver
        .require(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect_err("an unrecognised install is an error");

    assert!(matches!(error, Error::EmptyInstall(_)), "got {error:?}");
    let _ = server.join();
}

#[tokio::test]
async fn an_archive_that_is_not_the_declared_format_fails_as_an_install_error() {
    // A channel serving a zip where it promised a tarball. The download
    // succeeds and the unpacking is what fails.
    let scratch = tempfile::tempdir().unwrap();
    let archive = testing::single_root_archive("toolchain-1.0.0", ArchiveFormat::Zip);
    let digest = testing::digest(&archive);
    let (url, server) = testing::serve(archive, 1);

    let provider = installing_provider(&url, Some(&digest));
    let resolver = resolver_over(provider);

    let error = resolver
        .require(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect_err("a mislabelled archive cannot unpack");

    assert!(matches!(error, Error::Install { .. }), "got {error:?}");
    let _ = server.join();
}

#[tokio::test]
async fn an_unreachable_channel_fails_the_install_retryably() {
    let scratch = tempfile::tempdir().unwrap();
    let provider = installing_provider("http://127.0.0.1:1/archive", None);
    let resolver = resolver_over(provider);

    let error = resolver
        .require(&ResolveRequest::new(
            Language::nodejs(),
            settings(scratch.path()),
        ))
        .await
        .expect_err("an unreachable channel fails");

    assert!(matches!(error, Error::Download { .. }), "got {error:?}");
    assert!(error.is_retryable());
}

#[tokio::test]
async fn a_cache_root_that_cannot_be_listed_is_treated_as_empty() {
    // A host whose cache directory does not exist yet is every host, once.
    // Turning that into an error would break the first run on every machine.
    let scratch = tempfile::tempdir().unwrap();
    let not_a_directory = scratch.path().join("a-file");
    std::fs::write(&not_a_directory, b"x").unwrap();

    let mut settings = RuntimeSettings::new("1.0.0");
    settings.prefer_system = false;
    settings.cache_dir = not_a_directory.to_string_lossy().into_owned();

    let provider =
        Arc::new(StubProvider::new(Language::nodejs()).with_layout(layout("1.0.0", "/wherever")));
    let found = resolver_over(provider)
        .resolve(&ResolveRequest::probe(Language::nodejs(), settings))
        .await
        .expect("an unlistable cache is not an error");
    assert!(found.is_none());
}

#[tokio::test]
async fn a_cached_directory_the_provider_cannot_inspect_is_skipped() {
    // One unreadable leftover must not abort the scan and hide the install
    // sitting next to it.
    let scratch = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(scratch.path().join("toolchain-1.0.0")).unwrap();

    let mut settings = settings(scratch.path());
    settings.prefer_system = false;

    // The provider errors on every question, including `layout`.
    let found = resolver_over(Arc::new(DownProvider(Language::nodejs())))
        .resolve(&ResolveRequest::probe(Language::nodejs(), settings))
        .await;
    // `describe` fails first, so this is a provider failure rather than a scan
    // result — what matters is that it is reported rather than panicking.
    assert!(found.is_err());
}
