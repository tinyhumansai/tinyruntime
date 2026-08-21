//! Every type that crosses the tinyruntime boundary, and the names of the
//! members that carry them.
//!
//! `tinyruntime` ships as a loadable `TinyBus` module: `crates/tinyruntime` is
//! built as a `cdylib` and exports one object. A host that loads that binary can
//! call into it but cannot `use` anything out of it, so the payload vocabulary
//! has to be published as an ordinary library. This is that library.
//!
//! # The shape of the system
//!
//! Three kinds of peer share this contract.
//!
//! A **host** wants to run some JavaScript or some Python and does not want to
//! own a download pipeline, a version pin, or a pool of interpreter children to
//! get there. It calls [`names::INTERFACE`].
//!
//! The **router** — `crates/tinyruntime` — answers those calls. It owns
//! everything that is the same for every language: fetching an archive,
//! verifying its digest, unpacking it, promoting it into a cache atomically,
//! reusing it on the next start, and keeping a bounded set of warm worker
//! processes in front of it.
//!
//! A **provider** — `tinyruntime-nodejs`, `tinyruntime-python` — answers the
//! router's calls on [`names::PROVIDER_INTERFACE`]. It knows one language: which
//! host interpreters count as compatible, which archive to fetch for this
//! machine, where the binaries sit once unpacked, and what a warm worker for
//! that language looks like. It downloads nothing and installs nothing.
//!
//! That boundary is the reason this crate exists in the shape it does. Adding a
//! language should cost a release index and a path convention, not a fourth copy
//! of a download-verify-extract-install pipeline with its own bugs.
//!
//! # What is here
//!
//! - [`names`] — both interfaces, both object paths, one constant per member.
//! - [`language`] — the routing key.
//! - [`settings`] — what a host asks for, carried on every request.
//! - [`resolve`] — asking for a runtime and being told which one you got.
//! - [`provision`] — how a provider describes a toolchain it does not install.
//! - [`harness`] — the worker script a provider ships and the router runs.
//! - [`exec`] — running code, and what came back.
//! - [`pool`] — warm-worker tuning and counters.
//! - [`version`] — [`CONTRACT_VERSION`] and the [`is_compatible`] bind rule.
//!
//! # What is deliberately not here
//!
//! **No behavior.** No process is spawned, no byte is downloaded, and no path is
//! touched by anything in this crate. A payload type describes what a frame
//! carries, not what a module does with it.
//!
//! **No transport.** This crate does not depend on `tinybus` and holds no
//! connection, client, or codec. A host already owns its connection — its
//! reconnect policy, its timeouts, its tracing — and the useful part is the
//! vocabulary, not another wrapper around it.
//!
//! That is also a structural necessity, not only a preference: `tinybus` is
//! vendored as a submodule whose manifest inherits fields from its own nested
//! `[workspace.package]`. A crate that every workspace member can depend on has
//! to stay transport-free, and staying transport-free is what keeps this crate
//! down to two pure-Rust dependencies.
//!
//! # This crate sits underneath the implementations, not beside them
//!
//! `tinyruntime` **depends on this crate and re-exports all of it**, so
//! `tinyruntime::ExecRequest` and `tinyruntime_bus::exec::ExecRequest` are the
//! *same type*, not structural twins. Each provider crate does the same. One
//! definition, here, at the bottom — a parallel set of payload types for hosts
//! would mean a conversion at every call site that nothing checks.
//!
//! # Example
//!
//! Everything a host needs to ask for forty-two, spelled from constants:
//!
//! ```
//! use tinyruntime_bus::{ExecRequest, ExecResponse, Language, RuntimeSettings, names};
//!
//! let request = ExecRequest::new(
//!     Language::nodejs(),
//!     RuntimeSettings::new("v22.11.0"),
//!     "console.log(6 * 7)",
//! )
//! .with_cwd("/work/sandbox")
//! .with_timeout_ms(5_000);
//!
//! assert_eq!(names::methods::EXECUTE, "Execute");
//! assert_eq!(names::OBJECT_PATH, "/ai/tinyhumans/runtime/Runtime");
//! let body = serde_json::to_value([&request])?;
//! assert_eq!(body[0]["language"], serde_json::json!("nodejs"));
//!
//! let reply: ExecResponse = serde_json::from_value(serde_json::json!({
//!     "stdout": "42\n",
//!     "stderr": "",
//!     "exit_code": 0,
//!     "timed_out": false,
//!     "elapsed_ms": 3,
//!     "queue_wait_ms": 0,
//!     "runtime_version": "22.11.0",
//! }))?;
//! assert!(reply.success());
//! assert_eq!(reply.stdout, "42\n");
//! # Ok::<(), serde_json::Error>(())
//! ```

pub mod exec;
pub mod harness;
pub mod language;
pub mod names;
pub mod pool;
pub mod provision;
pub mod resolve;
pub mod settings;
pub mod version;

pub use exec::{ExecRequest, ExecResponse};
pub use harness::{WORKER_PROTOCOL_VERSION, WorkerHarness};
pub use language::{Language, NODEJS, PYTHON};
pub use names::{
    INTERFACE, METHODS, OBJECT_PATH, PROVIDER_INTERFACE, PROVIDER_METHODS, PROVIDER_OBJECT_PATH,
};
pub use pool::{PoolSettings, PoolStats, PoolStatsResponse};
pub use provision::{
    ArchiveFormat, Distribution, LayoutRequest, LayoutResponse, ProviderDescriptor, RuntimeLayout,
};
pub use resolve::{
    LanguageStatus, LanguagesResponse, ResolveRequest, ResolveResponse, ResolvedRuntime,
    RuntimeSource,
};
pub use settings::RuntimeSettings;
pub use version::{CONTRACT_VERSION, is_compatible};
