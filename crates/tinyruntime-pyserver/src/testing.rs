//! Test support: a worker that is a shell script, not an interpreter.
//!
//! The launch runs `<bin> -u <script>`, so pointing the "interpreter" at `sh`
//! and the script at a few lines of POSIX shell gives a genuine child process
//! speaking the genuine protocol with nothing installed. (`sh -u` is just
//! `nounset`, so the scripts below never touch an unset variable.)
#![cfg(all(test, unix))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;
use std::time::Duration;

use crate::ServerLaunch;

/// The ready line of a healthy fake worker serving the backend `alpha`.
pub(crate) const READY: &str =
    r#"printf '%s\n' '{"ready":true,"protocol":1,"backends":["alpha"]}'"#;

/// A request loop answering every line with `{"n":1}` under the request's id.
pub(crate) const ECHO_LOOP: &str = r#"
while IFS= read -r line; do
  id=${line#*\"id\":\"}; id=${id%%\"*}
  printf '{"id":"%s","ok":true,"result":{"n":1}}\n' "$id"
done
"#;

/// Write `body` as a script under `dir` and describe a launch of it.
pub(crate) fn launch(dir: &Path, body: &str) -> ServerLaunch {
    let script = dir.join("worker.sh");
    std::fs::write(&script, body).unwrap();
    let mut launch = ServerLaunch::new(
        "/bin/sh".into(),
        script,
        vec!["alpha".to_string()],
        vec![("MARK".to_string(), dir.join("mark").display().to_string())],
    );
    launch.handshake_timeout = Duration::from_secs(10);
    launch.request_timeout = Duration::from_secs(10);
    launch
}

/// A healthy worker: ready line, then [`ECHO_LOOP`].
pub(crate) fn healthy(dir: &Path) -> ServerLaunch {
    launch(dir, &format!("{READY}\necho noise >&2\n{ECHO_LOOP}"))
}
