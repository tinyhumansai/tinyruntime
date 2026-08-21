//! The contract version, and the rule a host uses to decide whether it can bind
//! to a module that reports one.
//!
//! The version describes *this vocabulary*, not the crate: bump the major
//! component when a payload's wire form changes incompatibly or a member is
//! removed or renamed, and the minor component when a member or an optional
//! field is added. It is deliberately independent of the package version the
//! release workflow bumps, which tracks the shipped artifact.
//!
//! Both halves of the contract move together. A provider reports the version it
//! was built against in its [`crate::ProviderDescriptor`], and the router
//! refuses to route to one it cannot bind to — a provider that answers a
//! question the router did not ask is worse than a language that is simply
//! unavailable.

/// The wire contract version this crate defines.
pub const CONTRACT_VERSION: (u32, u32) = (1, 0);

/// Returns whether a peer holding [`CONTRACT_VERSION`] can bind to one
/// reporting `other`.
///
/// Compatibility is the ordinary semantic-version rule for a pre-release-free
/// contract: the majors must match, and the other side must be at least as new,
/// because a caller cannot use a member the other side does not serve.
///
/// # Examples
///
/// ```
/// # use tinyruntime_bus::{is_compatible, CONTRACT_VERSION};
/// assert!(is_compatible(CONTRACT_VERSION));
/// assert!(is_compatible((1, 4)));
/// assert!(!is_compatible((2, 0)));
/// assert!(!is_compatible((0, 9)));
/// ```
#[must_use]
pub fn is_compatible(other: (u32, u32)) -> bool {
    binds(CONTRACT_VERSION, other)
}

/// The bind rule with the local version supplied explicitly.
///
/// [`is_compatible`] is this function applied to [`CONTRACT_VERSION`]. It is
/// split out so the unit tests can exercise both directions of the comparison
/// without pinning them to whatever the shipped version happens to be.
fn binds(local: (u32, u32), other: (u32, u32)) -> bool {
    let (local_major, local_minor) = local;
    let (other_major, other_minor) = other;

    other_major == local_major && other_minor >= local_minor
}

#[cfg(test)]
mod test;
