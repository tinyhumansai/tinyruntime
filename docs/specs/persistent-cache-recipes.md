# Persistent cache recipes

A host may approve named absolute cache scopes at module load. WorkerPrepareCached
adds a known reservation, an unchanged worker plan and one declarative cache
recipe; existing members keep their arities. No scopes are approved by default.
Router contract 1.2 introduces this member; unchanged providers remain 1.0.
Legacy-cache adoption defaults to `strict`, including when the policy field is
omitted from an older serialized recipe. A host may explicitly select
`adopt_verified_legacy` during migration.

The recipe carries bounded explicit commands, router-owned file artifacts,
required relative entries, and exact readiness marker bytes. No language,
package, model or interpreter convention is compiled into the router. Approved
subprocess recipes are trusted native programs, not a model tool or filesystem
sandbox. They provision at the approved final cache location: moving a prepared
interpreter environment would break programs that embed absolute paths.

Before cache access, reject unapproved scopes, nonabsolute/root scopes, traversal,
credential-store components, invalid portable relative paths and oversized
recipes. Open each directory component without following links and keep directory
handles; router reads/writes/rename/cleanup remain handle relative. Revalidate
scope identity before commands and publication. Final required entries may be
symlinks only when the recipe explicitly requests entry existence; their targets
are never read by that check. Router artifacts/markers cannot be symlinks.

An exclusive OS file lock serializes the complete check/provision/publication
across processes sharing a scope. Lock acquisition polls without blocking a
runtime thread indefinitely and observes stop/deadline. No process-lifetime
cache-key table is retained. A router-owned digest sidecar identifies every
recipe field without changing caller marker bytes. Changed
steps/environment/artifacts invalidate readiness. An existing sidecar must
match exactly; a mismatch always rebuilds even when legacy adoption is enabled.
Adoption is considered only when the sidecar is absent and marker bytes, every
artifact byte and every required entry match exactly while holding the cache
lock. The router then publishes the sidecar without running preparation
commands. Unsafe sidecars are refused. Identical recipe identity, marker bytes,
required entries and artifact bytes constitute a cache hit: no preparation
command runs on a hit.

Stage bounded router-owned artifacts, identity and marker files under a unique
owned directory in the approved scope. Promote explicit artifacts for commands,
but publish the new identity and readiness marker only after every command has
succeeded, native groups have been reaped and required paths verified. A stale
marker is invalidated before work; the prior identity sidecar stays in place
until successful publication so a failed rebuild cannot become adoptable as
legacy. Failed offline legacy adoption leaves the original marker untouched.
Only explicit artifact paths and the module's staging files are mutable; failures
preserve user cache content and partially provisioned final environments. Cleanup
never recursively deletes the user's cache. Files added to staging by another
actor are not claimed and deleted; cleanup failures remain retryable.

Caller abandonment leaves a detached supervisor holding preparation, lock and
staging ownership. Stop/Shutdown acknowledge only after child groups, pipe tasks,
staging and lock have been released. No marker can publish after Shutdown returns.
A native cleanup failure retains resources for public Stop retry. Each known
reservation is one immutable attempt; identical retries observe its outcome.
A failed provisioning attempt can be stopped and retried under a fresh reservation,
without permanent admission exhaustion or permanent failed-cache state.

Tests use local native fixtures and filesystem races, covering cache hits,
missing/partial/stale markers, failed commands and lock files, parallel managers,
restart reuse, canceled preparations/replies, retry/reclamation, traversal and
symlink replacement. No package downloads or external services run in tests.
Python provider and TinyJuice recipes remain separate owner slices.
