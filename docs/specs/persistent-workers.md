# Generic persistent JSONL workers

The router manages trusted, declarative process plans without language literals,
package knowledge, or model algorithms. Python providers describe interpreter and
virtual-environment recipes; TinyJuice describes its compressor script/model
recipe. Those integrations are separate slices. The first slice implements real
reserve, prepare, start, request, status, stop and terminal shutdown operations.
It does not switch hosts or publish a release artifact.

Each caller first reserves an opaque process-local handle. Only idle reservations
expire, after thirty seconds; admission counts live slots rather than lifetime
identifiers. The monotonically issued identity cannot recreate retired resources.
Prepare installs bounded script data under a module-owned private directory and
executes a bounded list of explicit preparation commands. Start begins only once
the caller knows its handle. Commands inherit only the existing safe environment
allowlist plus explicit recipe environment. No implicit interpreter fallback.

Each resource has one detached supervisor, a bounded command queue, and an
out-of-band stop signal. Dropping a caller future cannot release native ownership.
Stop and shutdown acknowledge only after process-group termination, direct-child
reaping and pipe-task completion. Shutdown is terminal; hosts must call it before
ABI unload. Module drop signals the same cleanup path; a stopped Tokio runtime
cannot itself substitute for the explicit unload barrier.

The stdio ready/request/response and backend/server status vocabulary preserves
existing JSONL fields and serde defaults exactly. The contract owns these types;
legacy pyserver paths reexport them. Requests use caller-known monotonically
increasing operation numbers. The supervisor retains a bounded recent terminal
reply window; evicted/stale operation numbers are rejected without reexecution.
Concurrent out-of-order submissions remain supported within that window. A lost
reply is recoverable by retry while retained, and always safe from duplicate
execution afterward. Payloads, script bytes, environment, queued commands,
response lines and output-memory retention have explicit fixed upper bounds.

Startup timeout defaults to30s, request deadline60s, startup backoff300s.
Deadlines cover the whole operation, including junk or wrong-id responses.
A failed request resets native ownership and retries once, preserving legacy
behavior. Successful requests reset idle age; configured backend-specific idle
rules rebuild the worker on the next start/request. Backend order stays stable.
Sensitive stdout/stderr, parameters, environment values and filesystem paths do
not enter public operational error messages or logs.

New router members advance the router contract minor version. Existing provider
members remain version1.0-compatible; router-only additions do not require an old
provider to expose unrelated operations. Existing member arities remain unchanged.
Provider descriptors omit an empty capability list, preserving the original
wire shape. `PrepareEnvironment` is optional and is called only when the
descriptor advertises `prepare_environment`; it returns native setup commands
and the prepared executable without executing them. Each command can carry an
optional deadline. Four hours bounds a whole cache recipe and thirty minutes
bounds each individual command.
