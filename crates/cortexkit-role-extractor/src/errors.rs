//! Error codes, their classes, and the two error shapes of `extract.run`.
//!
//! The vocabulary, its classes and both shapes are the shared model-batch
//! rules (`cortexkit-model-batch`, its `RULES.md` §14), re-exported here.
//! What is this role's own is which codes it answers with:
//! [`ADMISSION_CODES`] and [`ITEM_CODES`], the contract's §7 tables.

pub use cortexkit_model_batch::errors::{
    admission_class, classes, is_batch_level, item_class, ErrorClass, ItemError, ItemErrorDetail,
    Refusal, RefusalDetail, ACCOUNT_LIMIT_EXHAUSTED, ACCOUNT_NOT_ENTITLED, ACCOUNT_WALLED,
    ANSWER_TOO_LARGE, AUTH_FAILED, BATCH_ID_REUSE, BATCH_IN_PROGRESS, BATCH_LEVEL_CODES,
    COST_EXCEEDED, IMAGE_SOURCE_UNAVAILABLE, IMAGE_UNAVAILABLE, INVALID_ITEM, INVALID_PARAMS,
    MODEL_NOT_EXTRACTOR, MODEL_UNAVAILABLE, MODEL_UNKNOWN, PROVIDER_ERROR, PROVIDER_REFUSED,
    RATE_LIMITED,
};

/// Every admission refusal code `extractor/v1` answers with, in the
/// contract's §7 order. `cost_exceeded` is not one: this role's spend guard
/// works per item (§6.9). The two image codes come only from a module that
/// declares `image_reference`.
pub const ADMISSION_CODES: &[&str] = &[
    INVALID_PARAMS,
    BATCH_ID_REUSE,
    MODEL_UNKNOWN,
    MODEL_NOT_EXTRACTOR,
    ACCOUNT_WALLED,
    BATCH_IN_PROGRESS,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
    ACCOUNT_LIMIT_EXHAUSTED,
    ACCOUNT_NOT_ENTITLED,
    IMAGE_UNAVAILABLE,
    IMAGE_SOURCE_UNAVAILABLE,
];

/// Every per-item error code `extractor/v1` answers with, in the
/// contract's §7 order.
pub const ITEM_CODES: &[&str] = &[
    RATE_LIMITED,
    PROVIDER_ERROR,
    PROVIDER_REFUSED,
    INVALID_ITEM,
    COST_EXCEEDED,
    ANSWER_TOO_LARGE,
    AUTH_FAILED,
    MODEL_UNAVAILABLE,
    ACCOUNT_LIMIT_EXHAUSTED,
    ACCOUNT_NOT_ENTITLED,
];

#[cfg(test)]
mod tests {
    use super::*;
    use cortexkit_model_batch::errors::{ADMISSION_VOCABULARY, ITEM_VOCABULARY};

    #[test]
    fn every_code_is_in_the_shared_vocabulary_with_a_class() {
        for code in ADMISSION_CODES {
            assert!(ADMISSION_VOCABULARY.contains(code), "{code}");
            assert!(admission_class(code).is_some(), "{code}");
        }
        for code in ITEM_CODES {
            assert!(ITEM_VOCABULARY.contains(code), "{code}");
            assert!(item_class(code).is_some(), "{code}");
        }
        assert!(!ADMISSION_CODES.contains(&COST_EXCEEDED));
        assert_eq!(
            admission_class(IMAGE_UNAVAILABLE),
            Some(ErrorClass::Permanent)
        );
        assert_eq!(
            admission_class(IMAGE_SOURCE_UNAVAILABLE),
            Some(ErrorClass::Transient)
        );
    }
}
