//! The bus identity of both halves of this contract: the router a host calls,
//! and the provider interface a language module implements.
//!
//! Nothing here is a string literal at a call site. A host names a member
//! through [`methods`] and the object through [`OBJECT_PATH`], so a rename is a
//! compile error in every consumer rather than a runtime "unknown method".
//!
//! # Two interfaces, one contract
//!
//! [`INTERFACE`] is what a host calls: resolve a language, install it, run
//! something. [`PROVIDER_INTERFACE`] is what the router calls: three questions
//! only a language module can answer. They ship in one crate because the router
//! is a consumer of the second exactly as a host is a consumer of the first, and
//! splitting them would let the halves drift.
//!
//! Every provider implements [`PROVIDER_INTERFACE`] — that is what makes them
//! interchangeable — and claims its own well-known bus name, because two peers
//! cannot hold the same one.
//!
//! Each provider therefore serves at its *own* object path, derived from that
//! bus name. That is not a choice: `tinybus_module!` builds a module's manifest
//! object path by replacing the dots in its bus name with slashes, and a module
//! that served somewhere else would ship a manifest that disagreed with the
//! object it actually exports. [`object_path_for`] applies the same derivation,
//! so the router can address a provider it was only told the bus name of.

/// The well-known interface name the router claims on the bus.
pub const INTERFACE: &str = "ai.tinyhumans.runtime.Runtime";

/// The object path the router serves its interface at.
pub const OBJECT_PATH: &str = "/ai/tinyhumans/runtime/Runtime";

/// The interface every language provider implements.
pub const PROVIDER_INTERFACE: &str = "ai.tinyhumans.runtime.Provider";

/// The object path a module claiming `bus_name` serves its interfaces at.
///
/// The same derivation `tinybus_module!` uses to build a module's manifest, so
/// what this returns is where that module's object actually is.
///
/// # Examples
///
/// ```
/// # use tinyruntime_bus::names;
/// assert_eq!(
///     names::object_path_for(names::providers::NODEJS),
///     names::providers::NODEJS_OBJECT_PATH,
/// );
/// assert_eq!(names::object_path_for(names::INTERFACE), names::OBJECT_PATH);
/// ```
#[must_use]
pub fn object_path_for(bus_name: &str) -> String {
    format!("/{}", bus_name.replace('.', "/"))
}

/// One constant per member of [`INTERFACE`].
pub mod methods {
    /// Lists every language the router can route to, and whether it currently
    /// can. Returns a [`crate::LanguagesResponse`].
    pub const LANGUAGES: &str = "Languages";

    /// Resolves a language runtime, installing it when the request allows.
    ///
    /// Takes a [`crate::ResolveRequest`] and returns a
    /// [`crate::ResolveResponse`].
    pub const RESOLVE: &str = "Resolve";

    /// Runs inline source on a language runtime, resolving it first.
    ///
    /// Takes an [`crate::ExecRequest`] and returns an [`crate::ExecResponse`].
    pub const EXECUTE: &str = "Execute";

    /// Reports every live worker pool's counters.
    ///
    /// Returns a [`crate::PoolStatsResponse`].
    pub const POOL_STATS: &str = "PoolStats";
}

/// One constant per member of [`PROVIDER_INTERFACE`].
pub mod provider_methods {
    /// Reports what this provider is and what it targets by default.
    ///
    /// Returns a [`crate::ProviderDescriptor`].
    pub const DESCRIBE: &str = "Describe";

    /// Looks for a compatible toolchain already on the host.
    ///
    /// Takes a [`crate::RuntimeSettings`] and returns a
    /// [`crate::LayoutResponse`] that is empty when the host has none.
    pub const DETECT_SYSTEM: &str = "DetectSystem";

    /// Picks the archive to install for this host and these settings.
    ///
    /// Takes a [`crate::RuntimeSettings`] and returns a
    /// [`crate::Distribution`].
    pub const SELECT_DISTRIBUTION: &str = "SelectDistribution";

    /// Reports where the executables are inside an extracted install.
    ///
    /// Takes a [`crate::LayoutRequest`] and returns a [`crate::LayoutResponse`]
    /// that is empty when the directory holds no toolchain the settings accept.
    pub const LAYOUT: &str = "Layout";

    /// Supplies the worker harness the router launches for this language.
    ///
    /// Returns a [`crate::WorkerHarness`].
    pub const HARNESS: &str = "Harness";
}

/// The well-known bus names the first-party providers claim.
///
/// A router is not limited to these — its module configuration maps any language
/// to any bus name — but a build that ships the first-party providers should not
/// have to spell them.
pub mod providers {
    /// The bus name the Node.js provider claims.
    pub const NODEJS: &str = "ai.tinyhumans.runtime.nodejs.Provider";

    /// The object path the Node.js provider serves at.
    pub const NODEJS_OBJECT_PATH: &str = "/ai/tinyhumans/runtime/nodejs/Provider";

    /// The bus name the Python provider claims.
    pub const PYTHON: &str = "ai.tinyhumans.runtime.python.Provider";

    /// The object path the Python provider serves at.
    pub const PYTHON_OBJECT_PATH: &str = "/ai/tinyhumans/runtime/python/Provider";
}

/// Every member of [`INTERFACE`], in the order the interface dispatches them.
///
/// `crates/tinyruntime` asserts its declared manifest methods against this list,
/// so the two cannot drift.
pub const METHODS: &[&str] = &[
    methods::LANGUAGES,
    methods::RESOLVE,
    methods::EXECUTE,
    methods::POOL_STATS,
];

/// Every member of [`PROVIDER_INTERFACE`], in dispatch order.
///
/// Each provider crate asserts its declared manifest methods against this list.
pub const PROVIDER_METHODS: &[&str] = &[
    provider_methods::DESCRIBE,
    provider_methods::DETECT_SYSTEM,
    provider_methods::SELECT_DISTRIBUTION,
    provider_methods::LAYOUT,
    provider_methods::HARNESS,
];

#[cfg(test)]
mod test;
