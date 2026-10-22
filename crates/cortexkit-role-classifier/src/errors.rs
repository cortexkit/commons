//! Error codes, their classes, and the two error shapes of `classify.run`.
//!
//! An admission refusal answers the whole request with an `ERROR` frame
//! whose body is `{code, message, detail}` ([`Refusal`]); it writes nothing
//! and dispatches nothing. After admission the batch never fails as a
//! whole: an item that could not be answered carries an [`ItemError`] in
//! its place in the reply.
//!
//! Codes are open strings on the wire: a code this crate does not list is
//! kept as it was sent. Classes decode open too ([`ErrorClass::Other`]); a
//! caller that meets an unknown class treats it as permanent
//! ([`ErrorClass::effective`]), which is the safe direction.
//!
//! The vocabulary, its classes and both error shapes are the shared
//! model-batch rules (`cortexkit-model-batch`, its `RULES.md` §14),
//! re-exported here at their `classifier/v1` paths. What is this role's
//! own is which codes it answers with: [`ADMISSION_CODES`] and
//! [`ITEM_CODES`], the contract's §7 tables, and [`BATCH_LEVEL_CODES`],
//! the Workers AI account codes its §8 maps clef's errors to.

pub use cortexkit_model_batch::errors::{
    admission_class, classes, is_batch_level, item_class, ErrorClass, ItemError, ItemErrorDetail,
    Refusal, RefusalDetail, ACCOUNT_LIMIT_EXHAUSTED, ACCOUNT_NOT_ENTITLED, ACCOUNT_WALLED,
    ANSWER_TOO_LARGE, AUTH_FAILED, BATCH_ID_REUSE, BATCH_IN_PROGRESS, BATCH_LEVEL_CODES,
    COST_EXCEEDED, INVALID_ITEM, INVALID_PARAMS, MODEL_NOT_CLASSIFIER, MODEL_UNAVAILABLE,
    MODEL_UNKNOWN, PROVIDER_ERROR, PROVIDER_REFUSED, RATE_LIMITED,
};

/// Every admission refusal code in the contract's §7 table, in its order.
/// The batch-level account codes, which refuse a call only when its first
/// provider call meets one, are [`BATCH_LEVEL_CODES`].
pub const ADMISSION_CODES: &[&str] = &[
    INVALID_PARAMS,
    BATCH_ID_REUSE,
    MODEL_UNKNOWN,
    MODEL_NOT_CLASSIFIER,
    COST_EXCEEDED,
    ACCOUNT_WALLED,
    BATCH_IN_PROGRESS,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
];

/// Every per-item error code in the contract's §7 table, in its order.
/// `cost_exceeded`, `auth_failed` and `model_unavailable` are both
/// admission refusals and per-item errors. The batch-level account codes
/// are [`BATCH_LEVEL_CODES`]; `answer_too_large` is in the shared
/// vocabulary, but `classifier/v1` states no answer bound and never answers
/// with it.
pub const ITEM_CODES: &[&str] = &[
    RATE_LIMITED,
    PROVIDER_ERROR,
    PROVIDER_REFUSED,
    INVALID_ITEM,
    COST_EXCEEDED,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_code_has_its_tables_class() {
        for code in ADMISSION_CODES {
            assert!(admission_class(code).is_some(), "{code}");
        }
        for code in ITEM_CODES {
            assert!(item_class(code).is_some(), "{code}");
        }
        assert_eq!(
            admission_class(BATCH_IN_PROGRESS),
            Some(ErrorClass::Transient)
        );
        assert_eq!(admission_class(ACCOUNT_WALLED), Some(ErrorClass::Transient));
        assert_eq!(item_class(RATE_LIMITED), Some(ErrorClass::Transient));
        assert_eq!(item_class(PROVIDER_ERROR), Some(ErrorClass::Transient));
        assert_eq!(item_class(PROVIDER_REFUSED), Some(ErrorClass::Permanent));
        assert_eq!(item_class(INVALID_ITEM), Some(ErrorClass::Permanent));
        assert_eq!(item_class(COST_EXCEEDED), Some(ErrorClass::Permanent));
        for code in [AUTH_FAILED, MODEL_UNAVAILABLE] {
            assert_eq!(item_class(code), Some(ErrorClass::Permanent), "{code}");
            assert_eq!(admission_class(code), Some(ErrorClass::Permanent), "{code}");
        }
        assert_eq!(admission_class("nope"), None);
    }

    #[test]
    fn an_unknown_class_is_kept_and_acted_on_as_permanent() {
        let error: ItemError = serde_json::from_str(
            r#"{"code":"quota_gone","class":"degraded","message":"m","retry_after_ms":5}"#,
        )
        .unwrap();
        assert_eq!(error.code, "quota_gone");
        assert_eq!(error.class, ErrorClass::Other("degraded".into()));
        assert_eq!(error.class.effective(), ErrorClass::Permanent);
        assert!(!error.is_retried_on_resend());
        assert_eq!(
            serde_json::to_string(&error).unwrap(),
            r#"{"code":"quota_gone","class":"degraded","message":"m","retry_after_ms":5}"#
        );
    }

    #[test]
    fn stored_transient_errors_are_retried_and_permanent_ones_are_not() {
        for code in [RATE_LIMITED, PROVIDER_ERROR] {
            assert!(
                ItemError::for_code(code, "m").is_retried_on_resend(),
                "{code}"
            );
        }
        for code in [PROVIDER_REFUSED, INVALID_ITEM, COST_EXCEEDED] {
            assert!(
                !ItemError::for_code(code, "m").is_retried_on_resend(),
                "{code}"
            );
        }
    }

    #[test]
    fn auth_and_model_failures_are_never_stored_and_stop_the_call() {
        for code in [AUTH_FAILED, MODEL_UNAVAILABLE] {
            let error = ItemError::for_code(code, "m");
            assert!(!error.is_stored(), "{code}");
            assert!(error.stops_the_call(), "{code}");
        }
        assert!(ItemError::for_code(RATE_LIMITED, "m").stops_the_call());
        for code in [
            PROVIDER_ERROR,
            PROVIDER_REFUSED,
            INVALID_ITEM,
            COST_EXCEEDED,
        ] {
            let error = ItemError::for_code(code, "m");
            assert!(error.is_stored(), "{code}");
            assert!(!error.stops_the_call(), "{code}");
        }
        assert!(ItemError::for_code(RATE_LIMITED, "m").is_stored());
        assert_eq!(
            Refusal::model_unavailable("m").detail.model.as_deref(),
            Some("m")
        );
        assert_eq!(Refusal::auth_failed().detail.class, ErrorClass::Permanent);
    }

    #[test]
    fn the_batch_level_account_codes_are_never_stored_and_stop_the_call() {
        assert_eq!(
            BATCH_LEVEL_CODES,
            [ACCOUNT_LIMIT_EXHAUSTED, ACCOUNT_NOT_ENTITLED]
        );
        for code in BATCH_LEVEL_CODES {
            let error = ItemError::for_code(code, "m");
            assert!(!error.is_stored(), "{code}");
            assert!(error.stops_the_call(), "{code}");
            assert!(!ADMISSION_CODES.contains(code) && !ITEM_CODES.contains(code));
        }
        assert_eq!(
            item_class(ACCOUNT_LIMIT_EXHAUSTED),
            Some(ErrorClass::Transient)
        );
        assert_eq!(
            item_class(ACCOUNT_NOT_ENTITLED),
            Some(ErrorClass::Permanent)
        );
    }

    #[test]
    fn answer_too_large_is_in_the_vocabulary_but_not_the_tables() {
        assert_eq!(item_class(ANSWER_TOO_LARGE), Some(ErrorClass::Permanent));
        assert!(ItemError::for_code(ANSWER_TOO_LARGE, "m").is_stored());
        assert!(!ITEM_CODES.contains(&ANSWER_TOO_LARGE));
    }

    #[test]
    fn refusals_carry_their_class_and_field() {
        let refusal = Refusal::over_limit("items", 100, "too many items");
        assert_eq!(refusal.code, INVALID_PARAMS);
        assert_eq!(refusal.field(), Some("items"));
        assert_eq!(refusal.detail.limit, Some(100));
        assert_eq!(refusal.detail.class, ErrorClass::Permanent);
        let busy = Refusal::batch_in_progress(250);
        assert_eq!(busy.detail.class, ErrorClass::Transient);
        assert_eq!(busy.detail.retry_after_ms, Some(250));
    }
}
