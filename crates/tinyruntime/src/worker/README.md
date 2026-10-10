# Persistent JSONL worker ownership

`WorkerManager` accepts trusted, language-agnostic plans. It installs bounded
script bytes under its private cache directory and executes only the explicit
commands in the plan. It owns native process groups (Unix) or jobs (Windows),
stdio framing and cleanup. Language providers and application recipes decide
what those bytes mean; this module contains no interpreter/package/model logic.

Reserve a handle before preparation or startup. Idle reservations expire after
30 seconds. There are 16 live slots, with an eight-command queue for each
prepared resource. Retired identities cannot recreate native resources, and
repeated reserve/stop workflows do not consume permanent admission capacity.

Requests carry caller-known positive operation numbers. The eight-number replay
window supports concurrent out-of-order arrival within that window; identical
retries replay replies and conflicting retries fail. Older numbers return
`expired` without execution. Keep the original operation number after a lost
reply, and retrieve its outcome before advancing beyond the replay window.

A frame is at most 256 KiB, and one request consumes at most 1 MiB of response
frames, including ignored malformed/wrong-id lines. A script is at most 1 MiB;
plans have at most 16 preparation commands, each with at most 64 arguments and
64 environment entries totaling 64 KiB. Preparation captures/discards at most
1 MiB of combined stdout/stderr. Stderr draining retains only an 8 KiB buffer
and does not log payloads. Deadlines cover queued request time and startup.

Each resource has a detached supervisor. Losing a prepare/start/request reply
keeps cleanup reachable through the known handle. Stop uses an out-of-band
signal and acknowledges only after native termination, reaping and pipe-task
completion. A reap failure retains native ownership and returns `cleanup_failed`;
retry Stop rather than allocate around the unresolved resource. Ordinary command
failure does not poison Stop or admission after successful cleanup.

Shutdown is a terminal barrier and must succeed before ABI unload. Use per-handle
Stop for sign-out if the process will reuse the module. Manager drop signals
cleanup while the runtime is alive; abruptly stopping the runtime is not a
replacement for the explicit shutdown barrier.

Persistent cache preparation uses the separate `WorkerPrepareCached` member and
an absolute scope approved by the host at module load. No scope is approved by
default. The bounded recipe carries exact router-owned artifact bytes, trusted
native preparation commands, required entries, exact marker bytes and an
optional migration policy. The router holds a no-follow directory capability
and OS lock through preparation and publication. It never moves a prepared
environment from a staging path, because environment scripts may contain
absolute paths.

Cache reuse requires an exact recipe identity sidecar, marker, artifact bytes
and required entries. The default `strict` policy rebuilds a complete old
cache that has no sidecar. `adopt_verified_legacy` may publish an identity for
an identity-free cache only when all existing marker, artifact and required
entry checks pass under the lock; it runs no native commands. A mismatched
identity always rebuilds. Failed rebuilds preserve the previous identity, so
they cannot make a cache look legacy. See
[`persistent-cache-recipes.md`](../../../../docs/specs/persistent-cache-recipes.md)
for bounds and cleanup behavior.

The legacy JSONL ready/request/response and backend/server status types have one
definition in `tinyruntime-bus`; pyserver paths reexport those same definitions.
The old linked pyserver, Python preparation descriptions, TinyJuice recipes and
host switch remain separate migration slices. This implementation is a native
worker manager, not a claim that the entire product migration has shipped.
