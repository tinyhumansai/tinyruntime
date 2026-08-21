//! Unit tests for archive fetching and verification.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;

use reqwest::Client;
use sha2::{Digest, Sha256};
use tinyruntime_bus::{ArchiveFormat, Distribution, Language};

use super::{fetch, sanitise};
use crate::error::Error;

/// Serve `body` once over loopback and return the URL it is reachable at.
///
/// A real socket rather than a mock: the code under test streams a response body
/// chunk by chunk and hashes as it goes, and a mock that hands over one complete
/// buffer would not exercise that at all.
fn serve_once(body: Vec<u8>) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/archive", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if line == "\r\n" {
                break;
            }
            line.clear();
        }
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(header.as_bytes());
        let _ = stream.write_all(&body);
        let _ = stream.flush();
    });
    (url, handle)
}

fn distribution(url: &str) -> Distribution {
    Distribution::new("1.0.0", "toolchain.tar.gz", url, ArchiveFormat::TarGz)
}

#[tokio::test]
async fn writes_the_body_and_accepts_a_matching_digest() {
    let body = b"a plausible toolchain archive".to_vec();
    let digest = hex::encode(Sha256::digest(&body));
    let (url, server) = serve_once(body.clone());

    let scratch = tempfile::tempdir().unwrap();
    let target = scratch.path().join("toolchain.tar.gz");
    fetch(
        &Client::new(),
        &distribution(&url).with_sha256(digest),
        &target,
        &Language::nodejs(),
    )
    .await
    .expect("a verified archive downloads");

    assert_eq!(std::fs::read(&target).unwrap(), body);
    server.join().unwrap();
}

#[tokio::test]
async fn a_wrong_digest_is_refused_and_the_file_is_removed() {
    // Leaving a rejected archive on disk is the real hazard: the install path
    // reuses an archive it finds, so a bad one must not survive the refusal.
    let (url, server) = serve_once(b"not the published bytes".to_vec());

    let scratch = tempfile::tempdir().unwrap();
    let target = scratch.path().join("toolchain.tar.gz");
    let error = fetch(
        &Client::new(),
        &distribution(&url).with_sha256("00".repeat(32)),
        &target,
        &Language::nodejs(),
    )
    .await
    .expect_err("a mismatched archive is refused");

    assert!(
        matches!(error, Error::DigestMismatch { .. }),
        "got {error:?}"
    );
    assert!(
        !Path::new(&target).exists(),
        "the rejected archive was left behind"
    );
    server.join().unwrap();
}

#[tokio::test]
async fn a_digest_matches_case_insensitively() {
    // Channels publish hex in both cases; refusing uppercase would be a
    // spurious mismatch on an archive that is exactly right.
    let body = b"toolchain".to_vec();
    let digest = hex::encode(Sha256::digest(&body)).to_uppercase();
    let (url, server) = serve_once(body);

    let scratch = tempfile::tempdir().unwrap();
    fetch(
        &Client::new(),
        &distribution(&url).with_sha256(digest),
        &scratch.path().join("toolchain.tar.gz"),
        &Language::nodejs(),
    )
    .await
    .expect("uppercase hex is the same digest");
    server.join().unwrap();
}

#[tokio::test]
async fn an_undigested_channel_still_installs() {
    let body = b"unverifiable toolchain".to_vec();
    let (url, server) = serve_once(body.clone());

    let scratch = tempfile::tempdir().unwrap();
    let target = scratch.path().join("toolchain.tar.gz");
    fetch(
        &Client::new(),
        &distribution(&url),
        &target,
        &Language::python(),
    )
    .await
    .expect("a channel with no digest is usable, loudly");
    assert_eq!(std::fs::read(&target).unwrap(), body);
    server.join().unwrap();
}

#[tokio::test]
async fn an_unreachable_channel_reports_a_download_failure() {
    // Port 1 on loopback refuses rather than hanging, so this stays fast and
    // deterministic without a network.
    let scratch = tempfile::tempdir().unwrap();
    let error = fetch(
        &Client::new(),
        &distribution("http://127.0.0.1:1/archive"),
        &scratch.path().join("toolchain.tar.gz"),
        &Language::nodejs(),
    )
    .await
    .expect_err("an unreachable channel fails");
    assert!(matches!(error, Error::Download { .. }), "got {error:?}");
    assert!(error.is_retryable(), "a failed transfer is worth retrying");
}

#[test]
fn failure_messages_never_carry_the_url() {
    // These strings reach a host's UI and bug reports. A URL can carry a token
    // in a query string, so the message describes the kind of failure instead.
    let url = "https://channel.invalid/secret-token/archive.tar.gz";
    let error = Client::new().get(url).build().err();
    assert!(
        error.is_none(),
        "the URL itself is valid; only the request fails"
    );

    let message = sanitise(&timeout_error());
    assert!(!message.contains("channel.invalid"), "got `{message}`");
    assert!(!message.contains("secret-token"), "got `{message}`");
}

/// Produce a real timeout error to sanitise, without waiting for one.
fn timeout_error() -> reqwest::Error {
    // A zero timeout fails immediately with a timeout-classified error.
    let client = Client::builder()
        .timeout(std::time::Duration::from_nanos(1))
        .build()
        .unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async { client.get("http://127.0.0.1:1/").send().await })
        .expect_err("a one-nanosecond timeout always fires")
}
