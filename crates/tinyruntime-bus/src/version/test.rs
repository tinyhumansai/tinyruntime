//! Unit tests for the contract version bind rule.

use super::{CONTRACT_VERSION, binds, is_compatible};

#[test]
fn a_peer_on_the_same_version_binds() {
    assert!(binds((1, 2), (1, 2)));
    assert!(is_compatible(CONTRACT_VERSION));
}

#[test]
fn a_newer_minor_binds_because_it_still_serves_every_member() {
    assert!(binds((1, 2), (1, 7)));
}

#[test]
fn an_older_minor_does_not_bind() {
    assert!(
        !binds((1, 2), (1, 1)),
        "a caller cannot use a member the other side does not serve"
    );
}

#[test]
fn a_different_major_never_binds() {
    assert!(!binds((1, 0), (2, 0)));
    assert!(!binds((1, 0), (0, 9)));
    assert!(!binds((2, 0), (1, 9)));
}
