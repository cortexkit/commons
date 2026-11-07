//! Retry classification for answered routing refusals (CONTRACT §10).

use serde_json::Value;

/// Whether an answered routing refusal can be retried.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RetryClass {
    /// Retrying the same request is not expected to succeed later.
    Permanent,
    /// The refusal is temporary, without a known retry time.
    Transient,
    /// Retry at this absolute Unix timestamp in milliseconds.
    RetryAt(u64),
}

/// Classify a refusal using the normative mapping in CONTRACT §10.
///
/// Unknown codes and missing or malformed retry timestamps are permanent.
/// This function does not read the clock; callers can convert `RetryAt` into a
/// delay by saturating subtraction from their current Unix time in milliseconds.
pub fn retry_class(code: &str, detail: &Value) -> RetryClass {
    if let Some(at) = detail
        .as_object()
        .and_then(|detail| crate::errors::timed_wait_until_ms(code, detail))
    {
        RetryClass::RetryAt(at)
    } else if code == "account_selection_unavailable" {
        RetryClass::Transient
    } else {
        RetryClass::Permanent
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::{Refusal, RefusalCode};
    use serde_json::json;

    #[test]
    fn retry_class_covers_every_section_10_code() {
        let expected = [
            ("invalid_request", RetryClass::Permanent),
            ("invalid_requirements", RetryClass::Permanent),
            ("unknown_consumer", RetryClass::Permanent),
            ("consumer_not_owned", RetryClass::Permanent),
            ("pin_unavailable", RetryClass::RetryAt(200)),
            ("no_route", RetryClass::RetryAt(200)),
            ("no_engine", RetryClass::Permanent),
            ("unknown_decision", RetryClass::Permanent),
            ("report_conflict", RetryClass::Permanent),
            ("account_not_authorized", RetryClass::Permanent),
            ("auth_required", RetryClass::Permanent),
            ("account_hold", RetryClass::RetryAt(100)),
            ("account_quota_unknown", RetryClass::Permanent),
            ("account_pool_exhausted", RetryClass::RetryAt(200)),
            ("account_pool_reserve_limited", RetryClass::Permanent),
            ("account_selection_disabled", RetryClass::Permanent),
            ("account_selection_unavailable", RetryClass::Transient),
            ("admission_conflict", RetryClass::Permanent),
            ("pin_generation_conflict", RetryClass::Permanent),
            ("routing_unavailable", RetryClass::Permanent),
            ("routing_ambiguous", RetryClass::Permanent),
        ];
        let table = include_str!("../CONTRACT.md")
            .split_once("## 10. Refusal codes")
            .unwrap()
            .1
            .split_once("- [pinned]")
            .unwrap()
            .0;
        let codes: Vec<_> = table
            .lines()
            .filter_map(|line| line.strip_prefix("| `")?.split_once('`').map(|v| v.0))
            .collect();
        assert_eq!(codes, expected.map(|(code, _)| code));
        for (code, class) in expected {
            let detail = json!({"until_ms": 100, "reset_at_ms": 200});
            assert_eq!(retry_class(code, &detail), class, "{code}");
            let refusal = Refusal::with(RefusalCode::parse(code), "refused", &detail);
            assert_eq!(
                refusal.wait_until_ms(),
                match class {
                    RetryClass::RetryAt(at) => Some(at),
                    _ => None,
                },
                "{code}: retry classification must agree with timed status"
            );
        }
    }

    #[test]
    fn retry_class_rejects_missing_or_malformed_timestamps() {
        for detail in [
            Value::Null,
            json!({}),
            json!({"until_ms": "123", "reset_at_ms": "123"}),
            json!({"until_ms": -1, "reset_at_ms": -1}),
            json!({"until_ms": 1.5, "reset_at_ms": 1.5}),
        ] {
            for code in [
                "account_hold",
                "pin_unavailable",
                "no_route",
                "account_pool_exhausted",
            ] {
                assert_eq!(retry_class(code, &detail), RetryClass::Permanent, "{code}");
                if detail.is_object() {
                    assert_eq!(
                        Refusal::with(RefusalCode::parse(code), "refused", &detail).wait_until_ms(),
                        None,
                        "{code}"
                    );
                }
            }
        }
    }

    #[test]
    fn retry_class_preserves_absolute_timestamp_boundaries() {
        for timestamp in [0, u64::MAX] {
            assert_eq!(
                retry_class("account_hold", &json!({"until_ms": timestamp})),
                RetryClass::RetryAt(timestamp)
            );
            for code in ["pin_unavailable", "no_route", "account_pool_exhausted"] {
                assert_eq!(
                    retry_class(code, &json!({"reset_at_ms": timestamp})),
                    RetryClass::RetryAt(timestamp),
                    "{code}"
                );
            }
        }
        assert_eq!(
            retry_class(
                "future_refusal",
                &json!({"until_ms": 123, "reset_at_ms": 456})
            ),
            RetryClass::Permanent
        );
    }
}
