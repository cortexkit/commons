//! Role failures retain their existing vocabulary and refusal shape. Serving
//! modules must enforce tool-admission checks before dispatching to a role.

use cortexkit_model_batch::errors as batch;
pub use cortexkit_model_batch::errors::{ErrorClass, ItemError, Refusal, RefusalDetail};
pub use cortexkit_role_classifier::errors as classifier;
pub use cortexkit_role_extractor::errors as extractor;

pub const SCOPE_REQUIRED: &str = "scope_required";
pub const FLOW_SCOPE_REQUIRED: &str = "flow_scope_required";
pub const COST_UNPRICEABLE: &str = "cost_unpriceable";
pub const OUTPUT_INVALID: &str = "output_invalid";
pub const NO_ROUTE: &str = "no_route";

/// Permanent pre-dispatch refusals common to all three tools.
pub const TOOL_ADMISSION_CODES: &[&str] = &[SCOPE_REQUIRED, FLOW_SCOPE_REQUIRED, COST_UNPRICEABLE];

/// Completion's applicable model-batch codes plus its two completion-only codes.
/// Batch identity and role-specific model-kind errors do not apply to completion.
pub const COMPLETE_CODES: &[&str] = &[
    batch::INVALID_PARAMS,
    batch::MODEL_UNKNOWN,
    batch::COST_EXCEEDED,
    batch::ACCOUNT_WALLED,
    batch::AUTH_FAILED,
    batch::MODEL_UNAVAILABLE,
    batch::ACCOUNT_LIMIT_EXHAUSTED,
    batch::ACCOUNT_NOT_ENTITLED,
    batch::RATE_LIMITED,
    batch::PROVIDER_ERROR,
    batch::PROVIDER_REFUSED,
    batch::INVALID_ITEM,
    batch::ANSWER_TOO_LARGE,
    OUTPUT_INVALID,
    NO_ROUTE,
];

/// The published class, or None for a code outside this vocabulary. Decoding
/// refusals remains open, just as it is in the roles.
pub fn class(code: &str) -> Option<ErrorClass> {
    if TOOL_ADMISSION_CODES.contains(&code) || [OUTPUT_INVALID, NO_ROUTE].contains(&code) {
        Some(ErrorClass::Permanent)
    } else {
        batch::admission_class(code).or_else(|| batch::item_class(code))
    }
}

/// Build a refusal without guessing an unknown code's class.
pub fn refusal(code: &str, message: impl Into<String>) -> Option<Refusal> {
    class(code).map(|class| Refusal::new(code, message).with_detail(RefusalDetail::new(class)))
}
