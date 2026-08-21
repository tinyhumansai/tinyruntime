//! A worker that is not an interpreter, for testing the pool without one.
//!
//! The pool's interesting behaviour — the handshake, warm reuse, recycling, the
//! dispatch tagging that keeps a job from running twice, saturation — needs a
//! real child process on the other end of a real socket. Requiring Node or
//! Python for that would make the suite depend on what happens to be installed,
//! and mocking the transport would test the mock rather than the framing.
//!
//! So the test binary re-executes *itself*. [`serves_as_a_worker_when_asked`] is
//! an ordinary test that does nothing, unless [`WORKER_MARKER`] is set in its
//! environment — in which case it connects back, completes the handshake, and
//! serves jobs until the pool disconnects. That gives a genuine child process,
//! a genuine socket, and no dependency on anything installed.
//!
//! # Driving a scenario
//!
//! A job's `code` is a directive rather than source. See [`Directive`].

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use super::protocol::{Handshake, JobRequest, JobResponse};
use super::worker::Launch;

/// Set in a worker's environment to make the test binary serve instead of test.
pub(crate) const WORKER_MARKER: &str = "TINYRUNTIME_TEST_WORKER";

/// What the fake worker should do with a job, spelled in the job's `code`.
///
/// A directive rather than source, because the point is to drive the *pool's*
/// paths — a reply, a failure, a silence — not to evaluate anything.
pub(crate) enum Directive<'a> {
    /// Reply with this text on stdout and a zero exit.
    Echo(&'a str),
    /// Reply with this text on stderr and a non-zero exit.
    Fail(&'a str),
    /// Reply reporting the job was aborted at its deadline.
    TimedOut,
    /// Reply with a harness-level error, as a worker that could not run the job.
    HarnessError(&'a str),
    /// Never reply, so the caller's hard deadline is what ends the wait.
    Hang,
    /// Exit without replying, closing the protocol stream mid-job.
    Die,
    /// Reply with a frame for a different job, then the real one. Exercises the
    /// skip-and-keep-reading path without resetting the deadline.
    Misaddressed(&'a str),
    /// Emit an unparseable line before the real reply.
    Noise(&'a str),
}

impl Directive<'_> {
    /// The `code` string that selects this directive.
    pub(crate) fn code(&self) -> String {
        match self {
            Self::Echo(text) => format!("echo:{text}"),
            Self::Fail(text) => format!("fail:{text}"),
            Self::TimedOut => "timeout".to_string(),
            Self::HarnessError(message) => format!("harness-error:{message}"),
            Self::Hang => "hang".to_string(),
            Self::Die => "die".to_string(),
            Self::Misaddressed(text) => format!("misaddressed:{text}"),
            Self::Noise(text) => format!("noise:{text}"),
        }
    }
}

/// A launch that runs this test binary as a worker.
pub(crate) fn launch(language: tinyruntime_bus::Language) -> Launch {
    let binary = std::env::current_exe().expect("a test binary has a path");
    Launch {
        language,
        binary,
        args: vec![
            "--exact".to_string(),
            "pool::fake_worker::test::serves_as_a_worker_when_asked".to_string(),
            "--nocapture".to_string(),
            "--test-threads=1".to_string(),
        ],
        env: vec![
            (WORKER_MARKER.to_string(), "1".to_string()),
            // Some libtest builds consult these; carrying them keeps the child
            // from behaving differently than the parent.
            (
                "RUST_BACKTRACE".to_string(),
                std::env::var("RUST_BACKTRACE").unwrap_or_default(),
            ),
        ],
        protocol_version: tinyruntime_bus::WORKER_PROTOCOL_VERSION,
        handshake_timeout: std::time::Duration::from_secs(20),
    }
}

/// Serve the protocol until the pool disconnects.
///
/// Runs in the re-executed child, never in the parent.
fn serve() {
    let address = std::env::var("TINYRUNTIME_PROTOCOL_ADDR").expect("the pool supplies an address");
    let token = std::env::var("TINYRUNTIME_PROTOCOL_TOKEN").ok();

    let stream = TcpStream::connect(address).expect("the pool is listening");
    let mut writer = stream.try_clone().expect("the socket clones");
    let mut reader = BufReader::new(stream);

    let handshake = Handshake {
        ready: true,
        protocol: Some(tinyruntime_bus::WORKER_PROTOCOL_VERSION),
        language: Some("test".to_string()),
        error: None,
        token,
    };
    send(&mut writer, &serde_json::to_string(&handshake).expect("encodes"));

    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<JobRequest>(trimmed) else {
            continue;
        };
        if !handle(&mut writer, &request) {
            break;
        }
    }
}

/// Act on one job. Returns `false` when the worker should stop serving.
fn handle(writer: &mut TcpStream, request: &JobRequest) -> bool {
    let (kind, payload) = request
        .code
        .split_once(':')
        .unwrap_or((request.code.as_str(), ""));

    let mut reply = JobResponse {
        id: Some(request.id.clone()),
        ok: true,
        stdout: String::new(),
        stderr: String::new(),
        exit_code: Some(0),
        timed_out: false,
        elapsed_ms: 1,
        error: None,
    };

    match kind {
        "echo" => reply.stdout = payload.to_string(),
        "fail" => {
            reply.stderr = payload.to_string();
            reply.exit_code = Some(1);
        }
        "timeout" => {
            reply.timed_out = true;
            reply.exit_code = None;
        }
        "harness-error" => {
            reply.ok = false;
            reply.error = Some(payload.to_string());
        }
        "hang" => {
            // Outlive any deadline a test sets, without leaking a thread past
            // the parent's lifetime — the pool kills the child on drop.
            std::thread::sleep(std::time::Duration::from_secs(120));
            return false;
        }
        "die" => return false,
        "misaddressed" => {
            let stray = JobResponse {
                id: Some(format!("{}-not-this-one", request.id)),
                stdout: "stray".to_string(),
                ..reply.clone()
            };
            send(writer, &serde_json::to_string(&stray).expect("encodes"));
            reply.stdout = payload.to_string();
        }
        "noise" => {
            send(writer, "this is not json");
            reply.stdout = payload.to_string();
        }
        _ => reply.stdout = request.code.clone(),
    }

    send(writer, &serde_json::to_string(&reply).expect("encodes"));
    true
}

/// Write one newline-terminated frame.
fn send(writer: &mut TcpStream, line: &str) {
    let _ = writer.write_all(line.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}

#[cfg(test)]
mod test {
    /// Serves the worker protocol when re-executed by the pool; a no-op
    /// otherwise.
    ///
    /// This is not really a test — it is the entry point the pool launches. As
    /// an ordinary test run it asserts the one thing worth asserting: that it
    /// does nothing unless asked.
    #[test]
    fn serves_as_a_worker_when_asked() {
        if std::env::var(super::WORKER_MARKER).is_ok() {
            super::serve();
            // The pool owns this child's lifetime; leaving normally would let
            // libtest print a summary onto the job's stdout.
            std::process::exit(0);
        }
        assert!(
            std::env::var("TINYRUNTIME_PROTOCOL_ADDR").is_err(),
            "a plain test run must not be talking to a pool"
        );
    }
}
