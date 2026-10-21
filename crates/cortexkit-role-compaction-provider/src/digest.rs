//! Digests and canonical encodings shared by notices and views: the content
//! digest of a message or part, the length-prefixed framing every digest in
//! this role uses, and canonical decimal parsing for header values.

#[cfg(test)]
pub(crate) use cortexkit_role_llm_runner::content_digest::sha256_hex;
pub use cortexkit_role_llm_runner::content_digest::{content_digest, is_sha256_hex};
use sha2::{Digest, Sha256};

/// Append one field of a canonical digest input: its length in bytes as an
/// 8-byte unsigned big-endian integer, then the bytes. Length-prefixing makes
/// the concatenation unambiguous without escaping, the same framing as
/// [`crate::bootstrap::source_sha256`].
pub(crate) fn frame(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

pub(crate) fn finish(hasher: Sha256) -> String {
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Parse a canonical decimal `u64`: ASCII digits only, no sign, and no
/// leading zero except the single digit `0`.
pub(crate) fn parse_decimal(value: &str) -> Option<u64> {
    let canonical = !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'));
    if canonical {
        value.parse().ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimals_are_canonical() {
        assert_eq!(parse_decimal("0"), Some(0));
        assert_eq!(parse_decimal("42"), Some(42));
        assert_eq!(parse_decimal("18446744073709551615"), Some(u64::MAX));
        for bad in [
            "",
            "01",
            "+1",
            "-1",
            " 1",
            "1 ",
            "1e3",
            "18446744073709551616",
        ] {
            assert_eq!(parse_decimal(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn content_digest_ignores_member_order() {
        let a: serde_json::Value =
            serde_json::from_str(r#"{"role":"user","content":"hi"}"#).unwrap();
        let b: serde_json::Value =
            serde_json::from_str(r#"{"content":"hi","role":"user"}"#).unwrap();
        assert_eq!(content_digest(&a), content_digest(&b));
        assert!(is_sha256_hex(&content_digest(&a)));
    }
}
