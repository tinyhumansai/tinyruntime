//! Asking for a runtime, and being told which one you got.
//!
//! Resolution has exactly two outcomes worth distinguishing — a toolchain, or
//! nothing yet — so [`ResolveResponse`] carries an option rather than making a
//! probe's empty answer look like a failure.

mod types;

pub use types::{
    LanguageStatus, LanguagesResponse, ResolveRequest, ResolveResponse, ResolvedRuntime,
    RuntimeSource,
};

#[cfg(test)]
mod test;
