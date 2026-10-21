//! The content identity used by transcript reads, compaction notices and
//! lineage descent, independent of the read's media representation.

use sha2::{Digest, Sha256};

/// Lowercase hexadecimal SHA-256 over supplied bytes. Payload refs pass
/// decoded payload bytes; content_digest passes RFC 8785-encoded JSON.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// SHA-256 over the full inline value's RFC 8785 (JCS) encoding. Message and
/// part identities never hash the descriptor replacing a payload on a read.
pub fn content_digest(value: &serde_json::Value) -> String {
    let canonical = serde_jcs::to_vec(value).expect("a JSON value always has a JCS encoding");
    sha256_hex(&canonical)
}
