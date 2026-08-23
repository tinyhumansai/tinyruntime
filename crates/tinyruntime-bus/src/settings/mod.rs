//! What a host asks for, carried on every request.
//!
//! Settings travel with the call rather than living in the module. That is what
//! makes the router stateless with respect to configuration: two hosts sharing
//! one loaded module can pin different versions, and a configuration change
//! takes effect on the next call rather than on the next reload.

mod types;

pub use types::RuntimeSettings;

#[cfg(test)]
mod test;
