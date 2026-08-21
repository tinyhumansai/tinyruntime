# Roadmap

What is deliberately not built yet, and what would have to be true before it is.

## Shipped

- The router: resolution order, download with digest verification, unpack,
  atomic promotion, cross-process install locking, and cache reuse.
- The warm worker pool: bounded concurrency, queue backpressure with load
  shedding, idle reaping, recycle-after-N, and dispatch-tagged failures so a job
  never runs twice.
- The provider contract, and bus-backed routing to provider modules.

## Next

**Package installation.** A provider reports `npm` and `pip` in its layout, but
nothing calls them yet. A host that wants dependencies installs them itself. The
open question is whether that belongs behind a member here or stays a host
concern — it is the first thing that would make the contract meaningfully wider,
so it should not be added casually.

**Concurrent resolution of distinct languages.** `Languages` asks each provider
in turn, which is right for a handful of entries and wrong for a dozen. Worth
doing when there are a dozen.

**A file-backed resolution memo.** Resolution is memoised per process, so a
restart re-probes the host even when nothing changed. Cheap, but not free on a
machine that starts many short-lived hosts.

## Not planned

**Compiling providers in.** Feature-gated backends would make this repository
know about languages again, which is the thing the split exists to prevent.

**A provider marketplace.** Which modules may be loaded is a build-time decision
for the host, not runtime discovery. A registry a server could add entries to
would be a remote-code-execution surface with a download step.
