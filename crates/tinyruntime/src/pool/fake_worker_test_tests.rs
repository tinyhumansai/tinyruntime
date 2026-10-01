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
        super::connect_and_serve(&address, None, super::Mode::Silent);
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
