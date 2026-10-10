//! Recipe hashing does not require an expanding serialization buffer.
#![allow(
    clippy::unwrap_used,
    reason = "local fixture assertions match existing test conventions"
)]
use super::*;
#[test]
fn hash_writer_accepts_bytes_and_flush_without_buffering() {
    let mut writer = HashWriter(Sha256::new());
    assert_eq!(writer.write(b"recipe").unwrap(), 6);
    writer.flush().unwrap();
    assert_eq!(writer.0.finalize(), Sha256::digest(b"recipe"));
}
