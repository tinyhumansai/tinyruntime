//! Helpers shared by the tests that need real bytes.
//!
//! Two things are worth building rather than mocking. A loopback HTTP server,
//! because the download path streams and hashes chunk by chunk and a mock
//! handing over one complete buffer would not exercise that. And real archives,
//! because the extractor's job is to be correct about formats that only a real
//! encoder produces.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

use tinyruntime_bus::ArchiveFormat;

/// Serve `body` to the next `requests` callers, then stop.
///
/// Returns the URL and the server thread. Join the thread to be sure the server
/// finished before a test's temporary directory goes away.
pub(crate) fn serve(body: Vec<u8>, requests: usize) -> (String, JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("loopback is available");
    let url = format!(
        "http://{}/archive",
        listener.local_addr().expect("the listener has an address")
    );

    let handle = std::thread::spawn(move || {
        let mut served = 0;
        for _ in 0..requests {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let Ok(clone) = stream.try_clone() else {
                break;
            };
            let mut reader = BufReader::new(clone);
            let mut line = String::new();
            // Read past the request headers so the client's write completes.
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
            if stream.write_all(header.as_bytes()).is_ok() && stream.write_all(&body).is_ok() {
                let _ = stream.flush();
                served += 1;
            }
        }
        served
    });

    (url, handle)
}

/// Build an archive holding one root directory with one executable file in it.
///
/// The shape every toolchain channel actually publishes, which is what the
/// extractor asserts on.
pub(crate) fn single_root_archive(root: &str, format: ArchiveFormat) -> Vec<u8> {
    match format {
        ArchiveFormat::TarGz => {
            let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
            tar_into(encoder, root).finish().expect("gzip finishes")
        }
        ArchiveFormat::TarXz => {
            let encoder = xz2::write::XzEncoder::new(Vec::new(), 1);
            tar_into(encoder, root).finish().expect("xz finishes")
        }
        ArchiveFormat::Zip => zip_archive(root),
        _ => panic!("no builder for this archive format"),
    }
}

/// Write the standard single-root tree into a tar over `writer`.
fn tar_into<W: Write>(writer: W, root: &str) -> W {
    let mut builder = tar::Builder::new(writer);
    let payload = b"#!/bin/sh\n";
    let mut header = tar::Header::new_gnu();
    header.set_size(payload.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    builder
        .append_data(&mut header, format!("{root}/bin/tool"), &payload[..])
        .expect("the entry is appended");
    builder.into_inner().expect("the tar finishes")
}

/// The same tree as a zip.
fn zip_archive(root: &str) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            format!("{root}/bin/tool"),
            zip::write::SimpleFileOptions::default().unix_permissions(0o755),
        )
        .expect("the entry starts");
    writer.write_all(b"#!/bin/sh\n").expect("the entry writes");
    writer.finish().expect("the zip finishes").into_inner()
}

/// The lowercase hex SHA-256 of `bytes`.
pub(crate) fn digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// Make every `tracing` macro in the crate actually evaluate its fields.
///
/// Without a subscriber, `tracing` short-circuits before formatting, so a log
/// line's field expressions never run. That is invisible until something in one
/// of them panics — a `Display` on a poisoned lock, an index into an empty
/// slice — at which point it takes down whichever task happened to be logging.
///
/// Installing a subscriber that enables everything and renders nothing keeps the
/// suite silent while still running those expressions.
pub(crate) fn evaluate_log_fields() {
    use std::sync::Once;

    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // A failure here means a subscriber is already installed, which is fine:
        // the point is that one exists, not that this one won.
        let _ = tracing::subscriber::set_global_default(EvaluateFields);
    });
}

/// A subscriber that enables every level and discards every record.
struct EvaluateFields;

impl tracing::Subscriber for EvaluateFields {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        // The whole point: saying yes is what makes the field expressions run.
        true
    }

    fn new_span(&self, _span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, _event: &tracing::Event<'_>) {}

    fn enter(&self, _span: &tracing::span::Id) {}

    fn exit(&self, _span: &tracing::span::Id) {}
}

#[cfg(test)]
mod test {
    use super::{EvaluateFields, evaluate_log_fields};

    #[test]
    fn the_subscriber_answers_every_call_a_span_would_make() {
        // `tracing` only reaches these when something opens a span. Nothing in
        // this crate does yet, so they are exercised directly — an unimplemented
        // arm here would take down the first code that ever adds one.
        use tracing::Subscriber as _;

        evaluate_log_fields();
        let subscriber = EvaluateFields;
        let id = tracing::span::Id::from_u64(1);

        subscriber.enter(&id);
        subscriber.record_follows_from(&id, &id);
        subscriber.exit(&id);
    }
}
