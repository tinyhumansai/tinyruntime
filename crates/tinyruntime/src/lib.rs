//! The runtime router: resolve a language, provision it if it is not there, and
//! run code on it.
//!
//! # What this crate is for
//!
//! Any host that wants to run a bit of JavaScript or Python ends up needing the
//! same unglamorous machinery: find a compatible interpreter, or download one;
//! verify what it downloaded; unpack it somewhere durable; notice next time that
//! it is already there; and then not pay tens of megabytes of resident memory
//! for every execution. That machinery is identical for every language and is
//! reimplemented, slightly differently and slightly wrongly, once per host.
//!
//! This crate is that machinery, once, behind a bus. It is language-agnostic:
//! everything it knows about Node.js or Python it learns by asking a provider
//! module five questions.
//!
//! # How it fits together
//!
//! ```text
//!   host ──Execute──► tinyruntime ──Describe/DetectSystem/SelectDistribution──► tinyruntime-nodejs
//!                        │  │                                                 └► tinyruntime-python
//!                        │  └── download · verify · unpack · promote · reuse
//!                        └───── warm worker pool · job framing · backpressure
//! ```
//!
//! The provider answers what to install and where its parts are. This crate does
//! the installing. That split is what makes adding a language cost a release
//! index and a path convention rather than a fourth copy of a download pipeline.
//!
//! # The parts
//!
//! - [`provider`] — the five questions, the routing table, and the bus-backed
//!   provider that makes routing real.
//! - [`resolve`] — reuse before download, install under a lock, promote with one
//!   rename.
//! - [`download`], [`archive`], [`store`] — fetch and verify, unpack, and land it
//!   safely.
//! - [`pool`] — warm workers, their framing, and the backpressure in front of
//!   them.
//! - [`exec`] — [`Engine`], which is all of the above in one object.
//! - [`config`] — the [`ModuleConfig`] a host supplies at load time to say which
//!   languages this router routes and where their providers are.
//!
//! Every payload type comes from [`tinyruntime_bus`] and is re-exported here, so
//! `tinyruntime::ExecRequest` and `tinyruntime_bus::ExecRequest` are the same
//! type rather than structural twins.
//!
//! # What this crate deliberately does not hold
//!
//! **No language knowledge.** There is no `node` and no `python` in here. A
//! grep that finds one is a bug: it means something that belongs in a provider
//! leaked into the router, and the next language will have to work around it.
//!
//! **No configuration.** Every request carries the settings it should be served
//! under. Two hosts sharing one loaded module can pin different versions, and a
//! configuration change takes effect on the next call rather than on the next
//! reload.
//!
//! # Example
//!
//! Building an engine over a routing table, without a bus in sight — which is
//! also how the engine is tested:
//!
//! ```no_run
//! use std::path::PathBuf;
//! use tinyruntime::{Engine, Registry};
//!
//! # fn example(client: reqwest::Client) {
//! let engine = Engine::new(Registry::new(), client, PathBuf::from("/tmp/harnesses"));
//! assert!(engine.registry().is_empty());
//! # }
//! ```

pub mod archive;
pub mod config;
pub mod download;
pub mod error;
pub mod exec;
pub mod pool;
pub mod provider;
pub mod resolve;
pub mod store;

mod tinybus_module;

pub use config::{ModuleConfig, ProviderRoute};
pub use error::{Error, Result};
pub use exec::Engine;
pub use pool::{LangPool, Pools};
pub use provider::{BusProvider, Provider, Registry, Route};
pub use resolve::Resolver;

// The wire contract, re-exported whole. A consumer takes one dependency rather
// than two, and the payload types it names here are the very types the module
// serves rather than copies of them.
pub use tinyruntime_bus::{
    ArchiveFormat, CONTRACT_VERSION, Distribution, ExecRequest, ExecResponse, INTERFACE, Language,
    LanguageStatus, LanguagesResponse, LayoutRequest, LayoutResponse, METHODS, NODEJS, OBJECT_PATH,
    PROVIDER_INTERFACE, PROVIDER_METHODS, PYTHON, PoolSettings, PoolStats, PoolStatsResponse,
    ProviderDescriptor, ResolveRequest, ResolveResponse, ResolvedRuntime, RuntimeLayout,
    RuntimeSettings, RuntimeSource, WORKER_PROTOCOL_VERSION, WorkerHarness, is_compatible, names,
    object_path_for,
};
