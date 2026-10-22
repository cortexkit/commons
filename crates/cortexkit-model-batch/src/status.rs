//! Mapping a provider's error to a per-item code, once the runner's own
//! retries are spent.
//!
//! [`item_code_for_status`] is the mapping by HTTP status alone, for a
//! provider that documents no error codes. [`workers_ai_item_code`] reads
//! Cloudflare Workers AI's error code first, and falls back to the status
//! mapping for a code it does not list. `RULES.md` §15 states both tables.

use crate::errors::{
    ACCOUNT_LIMIT_EXHAUSTED, ACCOUNT_NOT_ENTITLED, AUTH_FAILED, INVALID_ITEM, MODEL_UNAVAILABLE,
    PROVIDER_ERROR, RATE_LIMITED,
};

/// The per-item error code for a provider's HTTP status, for a provider
/// whose error body the runner does not read:
///
/// - 401 and 403: `auth_failed`;
/// - 404: `model_unavailable`;
/// - 429 and 529 (an "overloaded" status some providers use):
///   `rate_limited`;
/// - 408 and every 5xx: `provider_error`;
/// - every other 4xx (400, 413, 422 and the rest): `invalid_item`.
///
/// 401, 403 and 404 say nothing about the item, so they are never
/// `invalid_item`, which a caller reads as "drop this item". `None` for a
/// status that is not an error. Nothing maps to `provider_refused`.
pub fn item_code_for_status(status: u16) -> Option<&'static str> {
    match status {
        401 | 403 => Some(AUTH_FAILED),
        404 => Some(MODEL_UNAVAILABLE),
        429 | 529 => Some(RATE_LIMITED),
        408 | 500..=599 => Some(PROVIDER_ERROR),
        400..=499 => Some(INVALID_ITEM),
        _ => None,
    }
}

/// The Workers AI error codes this crate maps, with what each means in
/// Cloudflare's error reference.
pub mod workers_ai_codes {
    /// 400: the input data is invalid.
    pub const INVALID_DATA: u64 = 5004;
    /// 400: the request is incomplete.
    pub const INCOMPLETE_REQUEST: u64 = 3003;
    /// 400: no such model.
    pub const NO_SUCH_MODEL: u64 = 5007;
    /// 404: no such model.
    pub const MODEL_NOT_FOUND: u64 = 3042;
    /// 403: the model needs a paid plan.
    pub const NEEDS_PAID_PLAN: u64 = 5035;
    /// 403: the account has not accepted the model's agreement.
    pub const MODEL_AGREEMENT: u64 = 5016;
    /// 403: the account is blocked.
    pub const ACCOUNT_BLOCKED: u64 = 3023;
    /// 413: the request is too large.
    pub const REQUEST_TOO_LARGE: u64 = 3006;
    /// 408: the request timed out.
    pub const TIMEOUT: u64 = 3007;
    /// 408: the request was aborted.
    pub const ABORTED: u64 = 3008;
    /// 429: the account's daily free allocation is used up.
    pub const DAILY_ALLOCATION_USED: u64 = 3036;
    /// 429: the model is out of capacity.
    pub const OUT_OF_CAPACITY: u64 = 3040;

    /// Every code [`super::workers_ai_item_code`] maps by code.
    pub const ALL: &[u64] = &[
        INVALID_DATA,
        INCOMPLETE_REQUEST,
        NO_SUCH_MODEL,
        MODEL_NOT_FOUND,
        NEEDS_PAID_PLAN,
        MODEL_AGREEMENT,
        ACCOUNT_BLOCKED,
        REQUEST_TOO_LARGE,
        TIMEOUT,
        ABORTED,
        DAILY_ALLOCATION_USED,
        OUT_OF_CAPACITY,
    ];
}

/// The per-item error code for a Cloudflare Workers AI error: the HTTP
/// status and the first `errors[].code` of its reply, when it gave one.
///
/// A listed code decides, whatever the status:
///
/// | code | role code |
/// |---|---|
/// | 5004 invalid data, 3003 incomplete request | `invalid_item` |
/// | 5007, 3042 no such model | `model_unavailable` |
/// | 5035 paid plan, 5016 model agreement, 3023 account blocked | `account_not_entitled` |
/// | 3006 request too large | `invalid_item` |
/// | 3007 timeout, 3008 aborted | `provider_error` |
/// | 3036 daily free allocation used | `account_limit_exhausted` |
/// | 3040 out of capacity | `rate_limited` |
///
/// Any other code, or none, keeps [`item_code_for_status`]: a 403 without
/// an account code is `auth_failed`, any other 429 is `rate_limited`, any
/// 5xx is `provider_error`. A runner puts an unlisted code into the item
/// error's `message`, so a caller can still read it.
pub fn workers_ai_item_code(status: u16, code: Option<u64>) -> Option<&'static str> {
    use workers_ai_codes::*;
    if !(400..=599).contains(&status) {
        return None;
    }
    match code {
        Some(INVALID_DATA | INCOMPLETE_REQUEST | REQUEST_TOO_LARGE) => Some(INVALID_ITEM),
        Some(NO_SUCH_MODEL | MODEL_NOT_FOUND) => Some(MODEL_UNAVAILABLE),
        Some(NEEDS_PAID_PLAN | MODEL_AGREEMENT | ACCOUNT_BLOCKED) => Some(ACCOUNT_NOT_ENTITLED),
        Some(TIMEOUT | ABORTED) => Some(PROVIDER_ERROR),
        Some(DAILY_ALLOCATION_USED) => Some(ACCOUNT_LIMIT_EXHAUSTED),
        Some(OUT_OF_CAPACITY) => Some(RATE_LIMITED),
        _ => item_code_for_status(status),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_map_to_item_codes() {
        for status in [429, 529] {
            assert_eq!(item_code_for_status(status), Some(RATE_LIMITED));
        }
        for status in [408, 500, 502, 503] {
            assert_eq!(item_code_for_status(status), Some(PROVIDER_ERROR));
        }
        for status in [400, 413, 422, 409] {
            assert_eq!(item_code_for_status(status), Some(INVALID_ITEM));
        }
        for status in [401, 403] {
            assert_eq!(item_code_for_status(status), Some(AUTH_FAILED));
        }
        assert_eq!(item_code_for_status(404), Some(MODEL_UNAVAILABLE));
        assert_eq!(item_code_for_status(200), None);
    }

    #[test]
    fn workers_ai_codes_decide_before_the_status() {
        assert_eq!(
            workers_ai_item_code(429, Some(3036)),
            Some(ACCOUNT_LIMIT_EXHAUSTED)
        );
        assert_eq!(workers_ai_item_code(429, Some(3040)), Some(RATE_LIMITED));
        assert_eq!(workers_ai_item_code(429, None), Some(RATE_LIMITED));
        for code in [5035, 5016, 3023] {
            assert_eq!(
                workers_ai_item_code(403, Some(code)),
                Some(ACCOUNT_NOT_ENTITLED)
            );
        }
        assert_eq!(workers_ai_item_code(403, Some(9999)), Some(AUTH_FAILED));
        assert_eq!(
            workers_ai_item_code(400, Some(5007)),
            Some(MODEL_UNAVAILABLE)
        );
        assert_eq!(
            workers_ai_item_code(404, Some(3042)),
            Some(MODEL_UNAVAILABLE)
        );
        assert_eq!(workers_ai_item_code(413, Some(3006)), Some(INVALID_ITEM));
        assert_eq!(workers_ai_item_code(408, Some(3008)), Some(PROVIDER_ERROR));
        assert_eq!(workers_ai_item_code(200, Some(3036)), None);
    }
}
