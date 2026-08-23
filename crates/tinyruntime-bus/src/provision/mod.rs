//! The vocabulary a provider uses to describe a toolchain it does not install.
//!
//! The split this family encodes is the whole point of the design. A provider
//! answers three language-shaped questions — which archive, laid out how, and
//! reported as which version — and the router does everything those answers
//! imply: fetch the bytes, verify the digest, unpack the archive, promote the
//! directory, and reuse it next time. Adding a language therefore costs a
//! release index and a path convention, not another copy of a download pipeline.

mod types;

pub use types::{
    ArchiveFormat, Distribution, LayoutRequest, LayoutResponse, ProviderDescriptor, RuntimeLayout,
};

#[cfg(test)]
mod test;
