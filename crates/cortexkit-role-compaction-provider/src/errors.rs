//! Codes this role names.
//!
//! Three kinds, kept apart because they reach different places:
//!
//! - a provider's refusal of a request, the `code` of an `ERROR` frame's
//!   body `{code, message, detail?}` ([`INVALID_PARAMS`], [`TRANSIENT`]).
//!   The runner treats any refusal of a step call as a failed call: it
//!   keeps the last applied CompactionMessage;
//! - the `code` of a `refuse` answer, which becomes the run's
//!   `provider_code` ([`refuse_codes`]);
//! - codes the runner answers or writes, quoted from `llm-runner/v1` so
//!   both sides spell them the same ([`runner_codes`]).
//!
//! Codes are open strings: a party that meets one it does not know treats
//! it as a terminal refusal of that one request.

use serde_json::Value;

/// A request field is malformed (an unknown preset or params value, say).
/// `detail.field` names it. The malformed-request code of the runner and
/// provider roles alike; `tool-provider/v1` spells it `invalid_request`.
pub const INVALID_PARAMS: &str = "invalid_params";

/// The provider could not answer now for a reason that may clear by itself.
/// Retryable within the call's budget.
pub const TRANSIENT: &str = "transient";

/// Every refusal code a provider of this role answers with.
pub const CODES: &[&str] = &[INVALID_PARAMS, TRANSIENT];

/// Whether a provider's refusal is retryable with the same request.
pub fn is_retryable(code: &str) -> bool {
    code == TRANSIENT
}

/// The field an `invalid_params` refusal names, or `None` for any other
/// code or a detail without `field`.
pub fn refused_field<'a>(code: &str, detail: Option<&'a Value>) -> Option<&'a str> {
    if code != INVALID_PARAMS {
        return None;
    }
    detail?.get("field")?.as_str()
}

/// Codes a `refuse` answer carries, which the runner writes as the run's
/// `provider_code`. A provider may use codes of its own beside these; for
/// these, `retryable` must be the value [`refuse_codes::retryable`] gives.
pub mod refuse_codes {
    /// The model's window cannot hold the smallest view the provider can
    /// build. The user has to switch to a larger model. Not retryable.
    pub const WINDOW_TOO_SMALL: &str = "window_too_small";
    /// The provider is busy with the session's history and cannot answer
    /// within the wait cap. Retryable: the next send may succeed.
    pub const PROVIDER_BUSY: &str = "provider_busy";
    /// The provider's configuration for this preset and params cannot be
    /// used. The user has to fix it. Not retryable.
    pub const MISCONFIGURED: &str = "misconfigured";
    /// The provider needed history it could not read (a gap after its
    /// cursor, and no transcript it can reach). Retryable.
    pub const HISTORY_UNREADABLE: &str = "history_unreadable";

    pub const ALL: &[&str] = &[
        WINDOW_TOO_SMALL,
        PROVIDER_BUSY,
        MISCONFIGURED,
        HISTORY_UNREADABLE,
    ];

    /// The retryability the role fixes for one of its own codes, or `None`
    /// for a code the provider defined itself, whose `retryable` value the
    /// provider chooses.
    pub fn retryable(code: &str) -> Option<bool> {
        match code {
            PROVIDER_BUSY | HISTORY_UNREADABLE => Some(true),
            WINDOW_TOO_SMALL | MISCONFIGURED => Some(false),
            _ => None,
        }
    }
}

/// Codes the runner answers or writes, taken from the `llm-runner/v1`
/// crate so both sides spell them the same.
pub mod runner_codes {
    use cortexkit_role_llm_runner::{compaction, errors};

    /// The runner's malformed-request refusal. At admission, a runner that
    /// does not declare the `compaction` group refuses a plan naming a
    /// compaction item with `detail.field` = [`PLAN_COMPACTION_ITEM`].
    pub const INVALID_PARAMS: &str = errors::INVALID_PARAMS;
    /// `detail.field` of that admission refusal: `plan.compaction_item`.
    pub const PLAN_COMPACTION_ITEM: &str = compaction::PLAN_COMPACTION_ITEM_FIELD;
    /// A `compaction.ready` whose route caller is not the provider at
    /// `plan.compaction_item.provider`, or for a session without one.
    pub const NOT_SESSION_COMPACTION_PROVIDER: &str = errors::NOT_SESSION_COMPACTION_PROVIDER;
    /// The codes a `compaction.ready` may be refused with.
    pub const READY_CODES: &[&str] = &[INVALID_PARAMS, NOT_SESSION_COMPACTION_PROVIDER];
    /// A `provider_code` the runner writes itself: a `wait` reached the
    /// runner's cap and the request could not be shown to fit.
    pub const COMPACTION_WAIT_EXCEEDED: &str = errors::provider_codes::COMPACTION_WAIT_EXCEEDED;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn codes_match_the_vectors() {
        let file = vectors::load("errors.json");
        let listed: Vec<&str> = vectors::cases(&file, "codes")
            .iter()
            .map(|case| case["code"].as_str().unwrap())
            .collect();
        for code in CODES {
            assert!(listed.contains(code), "{code} has no vector");
        }
        for case in vectors::cases(&file, "codes") {
            let code = case["code"].as_str().unwrap();
            assert_eq!(
                is_retryable(code),
                case["retryable"].as_bool().unwrap(),
                "{code}"
            );
        }
        let refuse: Vec<&str> = vectors::cases(&file, "refuse_codes")
            .iter()
            .map(|case| case["code"].as_str().unwrap())
            .collect();
        for code in refuse_codes::ALL {
            assert!(refuse.contains(code), "{code} has no vector");
        }
        for case in vectors::cases(&file, "refuse_codes") {
            let code = case["code"].as_str().unwrap();
            assert_eq!(
                refuse_codes::retryable(code),
                case["retryable"].as_bool(),
                "{code}"
            );
        }
    }

    #[test]
    fn refused_field_reads_only_invalid_params() {
        let detail = serde_json::json!({"field": "preset"});
        assert_eq!(refused_field(INVALID_PARAMS, Some(&detail)), Some("preset"));
        assert_eq!(refused_field(TRANSIENT, Some(&detail)), None);
        assert_eq!(refused_field(INVALID_PARAMS, None), None);
    }
}
