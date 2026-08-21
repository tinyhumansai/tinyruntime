//! Running code, and what came back.
//!
//! An execution request resolves and provisions as a side effect, so the common
//! case — "run this, I do not care where the interpreter came from" — is one
//! call. The reply keeps queue wait separate from run time, because a host that
//! cannot tell a slow job from a saturated pool will tune the wrong thing.

mod types;

pub use types::{ExecRequest, ExecResponse};

#[cfg(test)]
mod test;
