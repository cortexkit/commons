//! The `async` describe group: compaction off the runner's hot path
//! (CONTRACT.md §20).
//!
//! On a session where the provider declares `async` and the runner declares
//! the matching runner-side support in its `llm-runner/v1` describe (message
//! tags and the `context_ceiling` code), the runner never calls
//! `compaction.step`.
//! Instead:
//!
//! - after it writes, the runner publishes an advance notice on ck-bus
//!   ([`notice::AdvanceNotice`]). The bus carries ids, digests and headers,
//!   never message content, so the notice only wakes the provider, which then
//!   reads the new messages over its `llm-runner/v1` route with `session.read`;
//! - whenever it decides to, the provider publishes that a view is ready
//!   ([`notice::ViewReady`]); the runner fetches it with `view.get`
//!   ([`view::ViewGetRequest`]), checks its digest, persists it and applies it
//!   to every request until a newer one arrives ([`view::CompactionView`]);
//! - when the runner's prefix is being rewritten anyway, or the request has
//!   reached the Setup ceiling, it asks for a view synchronously with
//!   `compaction.ride` ([`ride::RideRequest`]);
//! - Setup's `ceiling` ([`ceiling::CeilingPolicy`]) says when the runner pauses
//!   for a view and, failing one, refuses the run with `context_ceiling`.

use sha2::{Digest, Sha256};

pub mod ceiling;
pub mod notice;
pub mod ride;
pub mod view;

#[cfg(test)]
mod vector_tests;

/// The describe group a provider lists in `role.describe.capabilities` when
/// it serves advance notices, views, `view.get`, `compaction.ride` and the
/// Setup ceiling as CONTRACT.md §20 specifies. A runner uses none of those,
/// and keeps calling `compaction.step`, unless the provider lists this group.
pub const GROUP: &str = "async";

/// The `llm-runner/v1` describe group a runner lists when it implements its
/// half of async mode. Defined by that role in `cortexkit-role-llm-runner`
/// 0.2.7; this constant quotes the spelling until this crate depends on that
/// release. A runner asks Setup for `compaction_mode: "async"` only when it
/// lists this group and the provider lists [`GROUP`].
pub const RUNNER_GROUP: &str = "compaction_async";

/// The ops a provider that declares [`GROUP`] must serve.
pub const GROUP_OPS: &[&str] = &[crate::ops::VIEW_GET, crate::ops::COMPACTION_RIDE];

/// The `llm-runner/v1` run-error code a runner writes when a request reached
/// the ceiling and no view brought it back under (CONTRACT.md §20.5). The
/// runner role defines it, with its `{window, size, cause}` detail, in
/// `cortexkit-role-llm-runner` 0.2.7; this constant quotes the spelling until
/// this crate depends on that release.
pub const CONTEXT_CEILING: &str = "context_ceiling";

/// The `cause` values of [`CONTEXT_CEILING`]'s detail, quoted from
/// `llm-runner/v1` like the code itself.
pub mod context_ceiling_causes {
    /// The ceiling wait ran out with no view that brought the request back
    /// under the ceiling.
    pub const VIEW_TIMEOUT: &str = "view_timeout";
    /// The `compaction.ride` call the runner makes at the ceiling could not
    /// reach the provider, and the current view does not bring the request
    /// back under the ceiling.
    pub const PROVIDER_UNAVAILABLE: &str = "provider_unavailable";
    /// The request is larger than the model's window even under the best view
    /// the runner holds, for example after a switch to a smaller window.
    pub const OVER_WINDOW: &str = "over_window";
}

/// Lowercase hexadecimal SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Whether `value` is a lowercase hexadecimal SHA-256 digest.
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

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

/// The digest of one message (or one message part) as the provider computes
/// it from what `session.read` returned: lowercase hexadecimal SHA-256 over
/// the value's RFC 8785 (JCS) encoding. JCS fixes member order and number
/// form, so the digest does not depend on how either side's JSON library
/// orders or spells the value. `through_digest` in an advance notice is this
/// digest of the message at `through_ordinal`.
pub fn content_digest(value: &serde_json::Value) -> String {
    let canonical = serde_jcs::to_vec(value).expect("a JSON value always has a JCS encoding");
    sha256_hex(&canonical)
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

    #[test]
    fn the_group_requires_both_ops() {
        assert_eq!(GROUP, "async");
        assert_eq!(GROUP_OPS, &["view.get", "compaction.ride"]);
    }
}
