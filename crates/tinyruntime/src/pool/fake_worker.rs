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
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;

use super::protocol::{Handshake, JobRequest, JobResponse};
use super::worker::Launch;

/// Set in a worker's environment to make the test binary serve instead of test.
///
/// The value selects how it misbehaves: `"1"` serves normally, `"silent"`
/// connects and closes without a handshake, and `"garbage"` sends something that
/// is not a handshake at all.
pub(crate) const WORKER_MARKER: &str = "TINYRUNTIME_TEST_WORKER";

/// Whether the worker should stay alive after its protocol stream closes.
///
/// The distinction matters to the pool: a parked worker whose *process* exited
/// is noticed before the next job is written, while one whose *socket* died with
/// the process still running is only discovered by the write failing. Those take
/// different paths, and both need a worker that behaves that way on purpose.
static LINGER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// How long the `hang` and `linger` directives keep a worker unresponsive.
///
/// Comfortably longer than the longest deadline any test sets — the pool's own
/// grace above a soft deadline is ten seconds — so the pool is always what ends
/// the wait. Deliberately not much longer than that: an unresponsive child is
/// still holding a process slot, and under a parallel run a two-minute sleep
/// turns a ten-second suite into a two-minute one.
const HANG_FOR: std::time::Duration = std::time::Duration::from_secs(25);

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
    /// Reply, then stop serving and exit — a worker whose process dies while
    /// parked between jobs.
    ExitAfterReply,
    /// Reply, then close the protocol stream but keep the process alive — a
    /// worker whose socket died without its process noticing.
    Linger,
    /// Write to the process's own stdout before replying, so the pool's drain of
    /// the child's file descriptors has something to read.
    Print(&'a str),
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
            Self::ExitAfterReply => "exit-after-reply".to_string(),
            Self::Linger => "linger".to_string(),
            Self::Print(text) => format!("print:{text}"),
        }
    }
}

/// A launch that runs this test binary as a worker misbehaving in `mode`.
pub(crate) fn launch_with_mode(language: tinyruntime_bus::Language, mode: &str) -> Launch {
    let mut launch = launch(language);
    launch.env.retain(|(name, _)| name != WORKER_MARKER);
    launch
        .env
        .push((WORKER_MARKER.to_string(), mode.to_string()));
    launch
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

/// Connect back to the pool and serve until it disconnects.
///
/// Runs in the re-executed child, never in the parent.
/// How a worker should behave once it has connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Speak the protocol properly.
    Serve,
    /// Connect and close without a handshake.
    Silent,
    /// Send something that is not a handshake at all.
    Garbage,
}

impl Mode {
    /// The mode a worker's marker value selects.
    pub(crate) fn of(marker: &str) -> Self {
        match marker {
            "silent" => Self::Silent,
            "garbage" => Self::Garbage,
            _ => Self::Serve,
        }
    }
}

fn serve() {
    let address = std::env::var("TINYRUNTIME_PROTOCOL_ADDR").expect("the pool supplies an address");
    let token = std::env::var("TINYRUNTIME_PROTOCOL_TOKEN").ok();
    let mode = Mode::of(&std::env::var(WORKER_MARKER).unwrap_or_default());
    connect_and_serve(&address, token, mode);
}

/// Connect to the pool and behave as `mode` says.
///
/// Split from [`serve`] because everything above it reads the process
/// environment, which a test cannot set — `unsafe` is forbidden workspace-wide.
/// Everything below it is the part worth checking, and a test can drive it
/// against a listener of its own.
pub(crate) fn connect_and_serve(address: &str, token: Option<String>, mode: Mode) {
    // Each connection starts without the flag: a spawned child serves exactly
    // one, but the in-process tests share a process, and a `linger` left set by
    // an earlier one would make the next sleep for two minutes.
    LINGER.store(false, std::sync::atomic::Ordering::SeqCst);
    let stream = TcpStream::connect(address).expect("the pool is listening");
    match mode {
        // Two ways to be a worker the pool must refuse, both of which a real
        // harness can be after a bad build.
        Mode::Silent => return,
        Mode::Garbage => {
            let mut writer = stream.try_clone().expect("the socket clones");
            send(&mut writer, "not a handshake at all");
            return;
        }
        Mode::Serve => {}
    }

    let writer = stream.try_clone().expect("the socket clones");
    serve_on(BufReader::new(stream), writer, token);

    if LINGER.load(std::sync::atomic::Ordering::SeqCst) {
        // The protocol stream is gone, but the process is not: the pool should
        // only find out when it writes the next job.
        std::thread::sleep(HANG_FOR);
    }
}

/// The protocol loop, over any duplex.
///
/// Split from [`serve`] so it can be driven in-process: the child's own
/// execution is not visible to coverage, and the directive handling is worth
/// testing directly rather than only through a spawned process.
pub(crate) fn serve_on(mut requests: impl BufRead, mut replies: impl Write, token: Option<String>) {
    let handshake = Handshake {
        ready: true,
        protocol: Some(tinyruntime_bus::WORKER_PROTOCOL_VERSION),
        language: Some("test".to_string()),
        error: None,
        token,
    };
    send(
        &mut replies,
        &serde_json::to_string(&handshake).expect("encodes"),
    );

    let mut line = String::new();
    loop {
        line.clear();
        match requests.read_line(&mut line) {
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
        if !handle(&mut replies, &request) {
            break;
        }
    }
}

/// Act on one job. Returns `false` when the worker should stop serving.
fn handle(writer: &mut impl Write, request: &JobRequest) -> bool {
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
            std::thread::sleep(HANG_FOR);
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
        "print" => {
            // The process's real stdout, not the reply. The pool drains it so a
            // chatty job cannot block on a full pipe.
            println!("{payload}");
            let _ = std::io::stdout().flush();
            reply.stdout = payload.to_string();
        }
        "exit-after-reply" => {
            send(writer, &serde_json::to_string(&reply).expect("encodes"));
            return false;
        }
        "linger" => {
            send(writer, &serde_json::to_string(&reply).expect("encodes"));
            LINGER.store(true, std::sync::atomic::Ordering::SeqCst);
            return false;
        }
        _ => reply.stdout = request.code.clone(),
    }

    send(writer, &serde_json::to_string(&reply).expect("encodes"));
    true
}

/// Write one newline-terminated frame.
fn send(writer: &mut impl Write, line: &str) {
    let _ = writer.write_all(line.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}

#[cfg(test)]
#[path = "fake_worker_test_tests.rs"]
mod test;
