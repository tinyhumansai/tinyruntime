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
mod test {
    use super::{Directive, JobRequest, handle, serve_on};

    /// One request line for `directive`.
    fn request_line(id: &str, directive: &Directive<'_>) -> String {
        format!(
            "{}\n",
            serde_json::to_string(&JobRequest {
                id: id.to_string(),
                code: directive.code(),
                cwd: None,
                timeout_ms: None,
            })
            .expect("encodes")
        )
    }

    /// Every reply frame `serve_on` wrote for `input`.
    fn drive(input: &str) -> Vec<serde_json::Value> {
        let mut replies = Vec::new();
        serve_on(
            std::io::BufReader::new(std::io::Cursor::new(input.to_string())),
            &mut replies,
            Some("token".to_string()),
        );
        String::from_utf8(replies)
            .expect("frames are utf-8")
            .lines()
            .map(|line| serde_json::from_str(line).expect("each frame is json"))
            .collect()
    }

    #[test]
    fn a_marker_value_selects_how_the_worker_misbehaves() {
        use super::Mode;

        assert_eq!(Mode::of("1"), Mode::Serve);
        assert_eq!(Mode::of(""), Mode::Serve);
        assert_eq!(Mode::of("silent"), Mode::Silent);
        assert_eq!(Mode::of("garbage"), Mode::Garbage);
    }

    #[test]
    fn the_handshake_comes_first_and_carries_the_secret() {
        let frames = drive("");
        assert_eq!(frames.len(), 1, "only the handshake should be sent");
        assert_eq!(frames[0]["ready"], serde_json::json!(true));
        assert_eq!(frames[0]["token"], serde_json::json!("token"));
        assert_eq!(
            frames[0]["protocol"],
            serde_json::json!(tinyruntime_bus::WORKER_PROTOCOL_VERSION)
        );
    }

    #[test]
    fn each_directive_produces_the_reply_it_names() {
        let echo = drive(&request_line("1", &Directive::Echo("out")));
        assert_eq!(echo[1]["stdout"], serde_json::json!("out"));
        assert_eq!(echo[1]["exit_code"], serde_json::json!(0));

        let fail = drive(&request_line("1", &Directive::Fail("bad")));
        assert_eq!(fail[1]["stderr"], serde_json::json!("bad"));
        assert_eq!(fail[1]["exit_code"], serde_json::json!(1));

        let timed_out = drive(&request_line("1", &Directive::TimedOut));
        assert_eq!(timed_out[1]["timed_out"], serde_json::json!(true));

        let harness = drive(&request_line("1", &Directive::HarnessError("nope")));
        assert_eq!(harness[1]["ok"], serde_json::json!(false));
        assert_eq!(harness[1]["error"], serde_json::json!("nope"));
    }

    #[test]
    fn a_misaddressed_directive_sends_a_stray_frame_before_the_real_one() {
        let frames = drive(&request_line("7", &Directive::Misaddressed("mine")));
        assert_eq!(frames.len(), 3, "handshake, stray, real");
        assert_eq!(frames[1]["id"], serde_json::json!("7-not-this-one"));
        assert_eq!(frames[2]["id"], serde_json::json!("7"));
        assert_eq!(frames[2]["stdout"], serde_json::json!("mine"));
    }

    #[test]
    fn a_noise_directive_emits_an_unparseable_line_before_the_reply() {
        let mut replies = Vec::new();
        serve_on(
            std::io::BufReader::new(std::io::Cursor::new(request_line(
                "1",
                &Directive::Noise("after"),
            ))),
            &mut replies,
            None,
        );
        let text = String::from_utf8(replies).expect("utf-8");
        assert!(text.contains("this is not json"));
        assert!(text.trim_end().ends_with('}'));
    }

    #[test]
    fn the_directives_that_change_how_the_process_ends_still_reply_first() {
        // `print`, `exit-after-reply`, and `linger` differ from `die` in that
        // the job *does* get an answer; only what happens to the process
        // afterwards differs. That reply is what a test at the pool level
        // asserts on, so it is worth checking here too.
        for directive in [
            Directive::Print("to-fd-one"),
            Directive::ExitAfterReply,
            Directive::Linger,
        ] {
            let frames = drive(&request_line("1", &directive));
            assert_eq!(
                frames.len(),
                2,
                "handshake and one reply for {:?}",
                directive.code()
            );
            assert_eq!(frames[1]["id"], serde_json::json!("1"));
            assert_eq!(frames[1]["ok"], serde_json::json!(true));
        }
    }

    #[test]
    fn a_silent_worker_connects_and_says_nothing() {
        use std::io::Read as _;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback");
        let address = listener.local_addr().expect("an address").to_string();
        let worker = std::thread::spawn(move || {
            super::connect_and_serve(&address, None, super::Mode::Silent)
        });

        let (mut stream, _) = listener.accept().expect("the worker connects");
        let mut said = String::new();
        stream.read_to_string(&mut said).expect("the stream closes");
        assert!(said.is_empty(), "a silent worker sent `{said}`");
        worker.join().expect("the worker finished");
    }

    #[test]
    fn a_garbage_worker_sends_something_that_is_not_a_handshake() {
        use std::io::Read as _;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback");
        let address = listener.local_addr().expect("an address").to_string();
        let worker = std::thread::spawn(move || {
            super::connect_and_serve(&address, None, super::Mode::Garbage);
        });

        let (mut stream, _) = listener.accept().expect("the worker connects");
        let mut said = String::new();
        stream.read_to_string(&mut said).expect("the stream closes");
        assert!(
            serde_json::from_str::<serde_json::Value>(said.trim()).is_err(),
            "a garbage worker sent valid json: `{said}`"
        );
        worker.join().expect("the worker finished");
    }

    #[test]
    fn a_serving_worker_completes_the_handshake_over_a_real_socket() {
        use std::io::{BufRead as _, BufReader, Write as _};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("loopback");
        let address = listener.local_addr().expect("an address").to_string();
        let worker = std::thread::spawn(move || {
            super::connect_and_serve(&address, Some("secret".to_string()), super::Mode::Serve);
        });

        let (stream, _) = listener.accept().expect("the worker connects");
        let mut writer = stream.try_clone().expect("the socket clones");
        let mut lines = BufReader::new(stream).lines();

        let handshake: serde_json::Value =
            serde_json::from_str(&lines.next().expect("a handshake").expect("readable"))
                .expect("the handshake is json");
        assert_eq!(handshake["token"], serde_json::json!("secret"));

        writer
            .write_all(request_line("1", &Directive::Echo("round-trip")).as_bytes())
            .expect("the request writes");
        let reply: serde_json::Value =
            serde_json::from_str(&lines.next().expect("a reply").expect("readable"))
                .expect("the reply is json");
        assert_eq!(reply["stdout"], serde_json::json!("round-trip"));

        drop(writer);
        drop(lines);
        worker.join().expect("the worker finished");
    }

    #[test]
    fn a_die_directive_stops_serving() {
        // The worker exits mid-job, which is what makes the pool's post-dispatch
        // path reachable.
        let input = format!(
            "{}{}",
            request_line("1", &Directive::Die),
            request_line("2", &Directive::Echo("never")),
        );
        let frames = drive(&input);
        assert_eq!(frames.len(), 1, "nothing should follow the handshake");
    }

    #[test]
    fn blank_and_unparseable_request_lines_are_skipped() {
        let input = format!(
            "\n   \nnot json\n{}",
            request_line("1", &Directive::Echo("survived"))
        );
        let frames = drive(&input);
        assert_eq!(frames[1]["stdout"], serde_json::json!("survived"));
    }

    #[test]
    fn an_unknown_directive_echoes_itself_back() {
        let mut replies = Vec::new();
        let request = JobRequest {
            id: "1".to_string(),
            code: "something-else".to_string(),
            cwd: None,
            timeout_ms: None,
        };
        assert!(handle(&mut replies, &request));
        let frame: serde_json::Value =
            serde_json::from_slice(&replies).expect("one frame with a trailing newline");
        assert_eq!(frame["stdout"], serde_json::json!("something-else"));
    }

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
