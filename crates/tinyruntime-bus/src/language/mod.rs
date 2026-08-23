//! The routing key every request carries.
//!
//! A [`Language`] names which provider serves a request. The router holds one
//! provider per language and nothing else about it, so the set of languages this
//! build can serve is decided by which provider modules are loaded rather than
//! by anything compiled in here.

mod types;

pub use types::{Language, NODEJS, PYTHON};

#[cfg(test)]
mod test;
