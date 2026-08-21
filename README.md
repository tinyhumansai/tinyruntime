# tinyruntime

The runtime router. A TinyBus module that resolves a language runtime, installs
one when the host has none, reuses one when it does, and runs code on a bounded
pool of warm interpreter processes.

## The problem it removes

Any host that wants to run a bit of JavaScript or Python ends up needing the
same unglamorous machinery: find a compatible interpreter, or download one;
verify what it downloaded; unpack it somewhere durable; notice next time that it
is already there; and then not pay tens of megabytes of resident memory for
every execution.

That machinery is identical for every language, and it gets reimplemented —
slightly differently and slightly wrongly — once per host. This is that
machinery, once, behind a bus.

## The split

```text
  host ──Execute──► tinyruntime ──Describe/DetectSystem/SelectDistribution──► tinyruntime-nodejs
                       │  │                                                 └► tinyruntime-python
                       │  └── download · verify · unpack · promote · reuse
                       └───── warm worker pool · job framing · backpressure
```

**This repository is the router.** It knows nothing about any language. It
downloads archives, checks digests, unpacks them, promotes them into a cache
atomically, finds them again on the next start, and keeps warm workers in front
of them.

**A provider module knows one language.** It answers five questions — what it
is, whether the host already has a usable toolchain, which archive to install
for this machine, where the binaries are once unpacked, and what a warm worker
looks like — and it downloads nothing and installs nothing.

Adding a language therefore costs a release index and a path convention, not a
fourth copy of a download pipeline with its own bugs.

## What a host calls

`ai.tinyhumans.runtime.Runtime`, at `/ai/tinyhumans/runtime/Runtime`:

| Member | Takes | Gives back |
| --- | --- | --- |
| `Languages` | — | every routed language and whether its provider is serving |
| `Resolve` | `ResolveRequest` | a toolchain, or nothing when a probe finds none |
| `Execute` | `ExecRequest` | stdout, stderr, exit code, and timings |
| `PoolStats` | — | every live pool's counters |

Every request carries the settings it should be served under, so the module
holds no configuration of its own: two hosts sharing one loaded module can pin
different versions, and a configuration change takes effect on the next call
rather than on the next reload.

```rust
use tinyruntime_bus::{ExecRequest, ExecResponse, Language, RuntimeSettings, names};

let request = ExecRequest::new(
    Language::nodejs(),
    RuntimeSettings::new("v22.11.0"),
    "console.log(6 * 7)",
)
.with_cwd("/work/sandbox")
.with_timeout_ms(5_000);

let reply: ExecResponse = proxy.call(names::methods::EXECUTE, (request,)).await?;
assert_eq!(reply.stdout, "42\n");
```

## Resolution order

Each step exists to avoid the cost of the next one:

1. **A resolution this process already made.** Free.
2. **A compatible toolchain on the host.** One `--version` probe. The common
   developer machine stops here and never downloads anything.
3. **A managed toolchain already in the cache.** A few `stat`s. This is what
   makes a warm restart free rather than a repeat download.
4. **A managed toolchain this call installs.** Hundreds of megabytes, once.

Steps 1 to 3 never touch the network, which is what makes a non-installing probe
worth calling: it answers "is this ready?" without committing anyone to a
download.

## Guarantees worth knowing

- **A digest mismatch is fatal, not a retry.** The bytes become an interpreter
  this host then runs code with. A verified archive installs; a mismatched one is
  deleted rather than left somewhere a later run might reuse it.
- **Two processes cannot install over each other.** An exclusive lock around the
  install directory makes the second one wait and then find the first one's work
  already there.
- **A failed upgrade keeps the working toolchain.** The install is promoted with
  one rename; a failure at that point restores what was there before.
- **A job never runs twice.** Worker failures are tagged by whether the job
  reached the worker. Only one that provably never left is retried.
- **A job cannot forge a protocol frame.** The worker protocol runs over an
  authenticated loopback socket, never over the job's own stdout.
- **A saturated pool sheds load rather than queueing without limit.** Callers are
  told not to fall back to spawning their own interpreter, which would
  reintroduce exactly the memory the pool caps.

## Layout

```text
crates/
├── tinyruntime-bus/    # the wire contract — what crosses the bus
└── tinyruntime/        # the router — behaviour, adapter, and the cdylib
vendor/tinybus/         # pinned TinyBus host types and module SDK
```

`crates/tinyruntime` re-exports the whole contract, so
`tinyruntime::ExecRequest` and `tinyruntime_bus::ExecRequest` are the same type.
A host that only makes calls takes `tinyruntime-bus` alone and compiles neither
the module nor `tinybus`.

## Building

```sh
git submodule update --init --recursive
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo build --all-targets --all-features
cargo test --all-features
```

See [`AGENTS.md`](AGENTS.md) for the working agreement, and
[`MODULE.md`](MODULE.md) for installing a release artifact.

## License

GPL-3.0-only. See [`LICENSE`](LICENSE).
