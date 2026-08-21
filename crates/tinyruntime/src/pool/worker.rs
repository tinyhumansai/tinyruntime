//! One warm interpreter child.
//!
//! A worker runs exactly one job at a time; concurrency comes from the pool
//! holding several. It stays alive between jobs — that is the entire point — and
//! is retired only when it has been idle too long, has served its job budget, or
//! has failed in a way that makes its framing untrustworthy.
//!
//! ## Why the protocol has its own socket
//!
//! The job's standard input, output, and error belong to the job. If the
//! protocol shared them, a job that printed a line shaped like a response frame
//! could answer its own request, and a job that read standard input could
//! consume the next request. So the router opens a loopback listener, passes its
//! address and a fresh secret to the child, and the harness connects back. The
//! child's own stdout is drained and logged, never parsed.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};
use tokio::net::TcpListener;
use tokio::process::{Child, Command};

use tinyruntime_bus::Language;

use super::protocol::{Handshake, JobRequest, JobResponse};

/// How long a freshly spawned worker has to complete its handshake by default.
///
/// Generous, because a cold interpreter on a loaded machine can take seconds and
/// a spurious timeout here costs a spawn rather than a job. [`Launch`] carries
/// its own so a caller with a tighter budget — a test, or a host that would
/// rather fail fast — can say so.
pub const DEFAULT_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything needed to spawn a worker for one language.
#[derive(Debug, Clone)]
pub struct Launch {
    /// The language, for logging and for checking the harness that answered.
    pub language: Language,
    /// The interpreter binary.
    pub binary: PathBuf,
    /// Arguments after the binary, ending with the harness script path.
    pub args: Vec<String>,
    /// The child's complete environment. Its own is cleared first.
    pub env: Vec<(String, String)>,
    /// The protocol version the harness claims to implement.
    pub protocol_version: u32,
    /// How long the worker has to connect back and hand over its handshake.
    pub handshake_timeout: Duration,
}

impl Launch {
    /// A fingerprint that decides whether a running pool can serve this launch.
    ///
    /// Changing the interpreter, its flags, or its environment means the warm
    /// workers are the wrong ones, and the pool is rebuilt rather than quietly
    /// answering from the old toolchain.
    #[must_use]
    pub fn fingerprint(&self) -> String {
        // The handshake budget is deliberately absent: it changes how long a
        // failing spawn takes, not which toolchain a warm worker is running, and
        // rebuilding a healthy pool over it would discard warm workers for
        // nothing.
        format!(
            "{}|{}|{:?}|{:?}|{}",
            self.language.as_str(),
            self.binary.display(),
            self.args,
            self.env,
            self.protocol_version
        )
    }
}

/// A failure from [`Worker::submit`], tagged with whether the job reached the
/// worker.
///
/// This tag is the only thing standing between a transient worker failure and a
/// job running twice. A write that failed means the bytes never arrived and the
/// job never ran; anything after that means it may have, and re-running it could
/// duplicate whatever it already did.
#[derive(Debug)]
pub struct SubmitFailure {
    /// What went wrong.
    pub reason: String,
    /// Whether the request reached the worker.
    pub dispatched: bool,
}

impl SubmitFailure {
    /// A failure before the request reached the worker. Safe to retry.
    fn pre(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            dispatched: false,
        }
    }

    /// A failure after the request reached the worker. Terminal.
    fn post(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            dispatched: true,
        }
    }
}

/// A warm interpreter child and its bookkeeping.
pub struct Worker {
    language: Language,
    child: Child,
    requests: Box<dyn AsyncWrite + Send + Unpin>,
    responses: Lines<BufReader<Box<dyn AsyncRead + Send + Unpin>>>,
    jobs_done: u64,
    last_used: Instant,
}

impl std::fmt::Debug for Worker {
    /// The two protocol streams are trait objects with no `Debug` of their own,
    /// and a worker's identity is what it is serving, not its file descriptors.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Worker")
            .field("language", &self.language)
            .field("jobs_done", &self.jobs_done)
            .finish_non_exhaustive()
    }
}

impl Worker {
    /// How many jobs this worker has served.
    #[must_use]
    pub fn jobs_done(&self) -> u64 {
        self.jobs_done
    }

    /// Spawn a worker and complete its handshake.
    ///
    /// # Errors
    ///
    /// Returns a description of what went wrong: the child could not be spawned,
    /// did not connect back, did not hand over the secret it was given, or
    /// implements a protocol version this build does not speak.
    pub async fn spawn(launch: &Launch) -> std::result::Result<Self, String> {
        tracing::info!(
            language = launch.language.as_str(),
            "[tinyruntime::pool] spawning a worker"
        );

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|error| {
            format!("the worker protocol listener could not be opened: {error}")
        })?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("the worker protocol address could not be read: {error}"))?;
        let token = uuid::Uuid::new_v4().to_string();

        let mut command = Command::new(&launch.binary);
        command.args(&launch.args);
        command.env_clear();
        for (name, value) in &launch.env {
            command.env(name, value);
        }
        command.env("TINYRUNTIME_PROTOCOL_ADDR", address.to_string());
        command.env("TINYRUNTIME_PROTOCOL_TOKEN", &token);
        // A job inherits end-of-file on standard input, matching what a
        // one-shot child would see, rather than a pipe it could read the next
        // request out of.
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        no_console_window(&mut command);

        let mut child = command
            .spawn()
            .map_err(|error| format!("the worker could not be started: {error}"))?;

        if let Some(stdout) = child.stdout.take() {
            drain(launch.language.clone(), stdout, "stdout");
        }
        if let Some(stderr) = child.stderr.take() {
            drain(launch.language.clone(), stderr, "stderr");
        }

        let (stream, _) = tokio::time::timeout(launch.handshake_timeout, listener.accept())
            .await
            .map_err(|_| "the worker did not connect back in time".to_string())?
            .map_err(|error| format!("the worker connection was refused: {error}"))?;
        let (reader, writer) = tokio::io::split(stream);
        let boxed: Box<dyn AsyncRead + Send + Unpin> = Box::new(reader);
        let mut responses = BufReader::new(boxed).lines();

        let handshake = read_handshake(&mut responses, launch.handshake_timeout).await?;
        verify_handshake(&handshake, launch, &token)?;

        tracing::info!(
            language = launch.language.as_str(),
            "[tinyruntime::pool] worker ready"
        );

        Ok(Self {
            language: launch.language.clone(),
            child,
            requests: Box::new(writer),
            responses,
            jobs_done: 0,
            last_used: Instant::now(),
        })
    }

    /// Send one job and wait for its reply.
    ///
    /// `hard_deadline` is a backstop above the job's own soft deadline: the
    /// worker aborts at the soft one and still replies, so this fires only when
    /// the worker itself has wedged. On any failure the caller must discard this
    /// worker — its framing can no longer be trusted.
    ///
    /// # Errors
    ///
    /// Returns a [`SubmitFailure`] carrying whether the job reached the worker.
    pub async fn submit(
        &mut self,
        request: &JobRequest,
        hard_deadline: Option<Duration>,
    ) -> std::result::Result<JobResponse, SubmitFailure> {
        let mut line = serde_json::to_string(request).map_err(|error| {
            SubmitFailure::pre(format!("the job could not be encoded: {error}"))
        })?;
        line.push('\n');

        // A write that fails means the bytes never reached the worker, so the
        // job never ran and a retry is safe.
        self.requests
            .write_all(line.as_bytes())
            .await
            .map_err(|error| SubmitFailure::pre(format!("the job could not be sent: {error}")))?;
        // Past here the request is in flight: the job may execute, so every later
        // failure is terminal.
        self.requests.flush().await.map_err(|error| {
            SubmitFailure::post(format!("the job could not be flushed: {error}"))
        })?;

        self.await_response(&request.id, hard_deadline).await
    }

    /// Read lines until the one answering `id` arrives.
    ///
    /// The deadline is fixed rather than reset per line, so skipping unparseable
    /// or mismatched frames cannot extend a wedged worker's grace indefinitely.
    async fn await_response(
        &mut self,
        id: &str,
        hard_deadline: Option<Duration>,
    ) -> std::result::Result<JobResponse, SubmitFailure> {
        let deadline = hard_deadline.map(|budget| tokio::time::Instant::now() + budget);

        loop {
            let next = match deadline {
                Some(at) => tokio::time::timeout_at(at, self.responses.next_line())
                    .await
                    .map_err(|_| {
                        SubmitFailure::post("the worker stopped answering and was discarded")
                    })?,
                None => self.responses.next_line().await,
            };

            let line = match next {
                Ok(Some(line)) => line,
                Ok(None) => {
                    return Err(SubmitFailure::post("the worker closed its protocol stream"));
                }
                Err(error) => {
                    return Err(SubmitFailure::post(format!(
                        "the worker's reply could not be read: {error}"
                    )));
                }
            };

            let Ok(response) = serde_json::from_str::<JobResponse>(&line) else {
                tracing::warn!(
                    language = self.language.as_str(),
                    "[tinyruntime::pool] skipped an unparseable worker line"
                );
                continue;
            };
            if response.id.as_deref() != Some(id) {
                tracing::debug!(
                    language = self.language.as_str(),
                    "[tinyruntime::pool] skipped a reply for another job"
                );
                continue;
            }

            self.jobs_done += 1;
            self.last_used = Instant::now();
            return Ok(response);
        }
    }

    /// Whether this worker's child has already exited.
    ///
    /// Checked before a parked worker is reused. Without it a worker that died
    /// while idle is only discovered *after* the job has been written to it —
    /// and because a TCP write into a closed peer's buffer succeeds, that
    /// discovery arrives at the read, which is tagged post-dispatch and never
    /// retried. The job provably never ran, so failing it would be wrong; this
    /// check is what turns that case back into a transparent respawn.
    ///
    /// It cannot be exhaustive — the child may die between this call and the
    /// write — and it is not meant to be. The remaining window stays correctly
    /// classified as terminal.
    pub fn has_exited(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(Some(_)) | Err(_))
    }

    /// Whether this worker has served its job budget. A budget of `0` disables
    /// recycling.
    #[must_use]
    pub fn should_recycle(&self, budget: u64) -> bool {
        budget > 0 && self.jobs_done >= budget
    }

    /// Whether this worker has been parked at least `ttl`.
    #[must_use]
    pub fn idle_expired(&self, ttl: Duration) -> bool {
        self.last_used.elapsed() >= ttl
    }

    /// Ask the child to exit. Best effort; dropping the worker kills it anyway.
    pub fn shutdown(mut self) {
        if let Err(error) = self.child.start_kill() {
            tracing::debug!(
                language = self.language.as_str(),
                "[tinyruntime::pool] a worker could not be signalled: {error}"
            );
        }
    }
}

/// Read the single handshake line, or say why it never came.
async fn read_handshake(
    responses: &mut Lines<BufReader<Box<dyn AsyncRead + Send + Unpin>>>,
    budget: Duration,
) -> std::result::Result<Handshake, String> {
    match tokio::time::timeout(budget, responses.next_line()).await {
        Ok(Ok(Some(line))) => serde_json::from_str(&line)
            .map_err(|error| format!("the worker's handshake could not be read: {error}")),
        Ok(Ok(None)) => Err("the worker exited before its handshake".to_string()),
        Ok(Err(error)) => Err(format!("the worker's handshake could not be read: {error}")),
        Err(_) => Err("the worker's handshake timed out".to_string()),
    }
}

/// Refuse a handshake that is not ready, not authentic, or not this protocol.
fn verify_handshake(
    handshake: &Handshake,
    launch: &Launch,
    token: &str,
) -> std::result::Result<(), String> {
    if !handshake.ready {
        return Err(format!(
            "the worker failed to start: {}",
            handshake.error.as_deref().unwrap_or("no reason given")
        ));
    }
    if handshake.token.as_deref() != Some(token) {
        // Anything can reach a loopback port. Only the child this router just
        // spawned was given the secret.
        return Err("the worker did not present the secret it was given".to_string());
    }
    match handshake.protocol {
        Some(version) if version == launch.protocol_version => {}
        Some(version) => {
            return Err(format!(
                "the worker speaks protocol {version}, not {}",
                launch.protocol_version
            ));
        }
        None => return Err("the worker did not say which protocol it speaks".to_string()),
    }
    Ok(())
}

/// Continuously read and log a child stream so a chatty job never blocks on a
/// full pipe.
///
/// Deliberately never parsed as protocol: this is the job's own output, and
/// treating it as framing is exactly the confusion the separate socket exists to
/// prevent.
fn drain(
    language: Language,
    stream: impl tokio::io::AsyncRead + Send + Unpin + 'static,
    which: &'static str,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stream).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::trace!(
                language = language.as_str(),
                "[tinyruntime::pool] worker {which}: {line}"
            );
        }
    });
}

/// Suppress the console window Windows would otherwise flash for every worker.
#[cfg(windows)]
fn no_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

/// No-op off Windows.
#[cfg(not(windows))]
fn no_console_window(_command: &mut Command) {}

#[cfg(test)]
#[path = "worker_test.rs"]
mod test;
