//! Codes this role names.
//!
//! Two kinds, kept apart because they reach different places:
//!
//! - a provider's refusal of a request, the `code` of an `ERROR` frame's
//!   body `{code, message, detail?}`. Any refusal of a hook call makes the
//!   hook unavailable for that call, and the subscription's
//!   `on_unavailable` decides what happens;
//! - codes and reasons the runner answers or writes, quoted from
//!   `llm-runner/v1` so both sides spell them the same ([`runner_codes`]).
//!
//! Codes are open strings: a party that meets one it does not know treats
//! it as a terminal refusal of that one request.

use serde_json::Value;

/// A request field is malformed (an unknown preset or params value, say).
/// `detail.field` names it. The malformed-request code of the runner and
/// provider roles alike; `tool-provider/v1` spells it `invalid_request`.
pub const INVALID_PARAMS: &str = "invalid_params";

/// A hook call for a hook and phase the provider did not declare for the
/// call's preset and params. `detail` carries `hook` and, on `pre_tool`,
/// `phase`.
pub const NOT_SUBSCRIBED: &str = "not_subscribed";

/// The provider could not answer now for a reason that may clear by itself.
/// Retryable, but only within the hook's budget.
pub const TRANSIENT: &str = "transient";

/// Every refusal code a provider of this role answers with.
pub const CODES: &[&str] = &[INVALID_PARAMS, NOT_SUBSCRIBED, TRANSIENT];

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

/// Codes and reasons the runner answers or writes, taken from the
/// `llm-runner/v1` crate so both sides spell them the same.
pub mod runner_codes {
    use cortexkit_role_llm_runner::errors;

    /// The runner's malformed-request refusal. At admission it refuses a
    /// plan whose step-transform subscription is malformed, covers tools or
    /// ops the provider's declaration does not, or gives `replace` on
    /// `pre_user` or `post_assistant` to a provider that is not the
    /// reduction owner, with `detail.field` = [`PLAN_STEP_TRANSFORM_ITEMS`]
    /// (`subscription::InvalidSubscriptionDetail`).
    pub const INVALID_PARAMS: &str = errors::INVALID_PARAMS;
    /// `detail.field` of that admission refusal. The runner role does not
    /// name it yet; see the contract's Appendix A.
    pub const PLAN_STEP_TRANSFORM_ITEMS: &str = "plan.step_transform_items";
    /// The runner's admission refusal for a plan its providers' current
    /// declarations no longer cover: a subscription whose hook or preset is
    /// gone, or whose frozen `on_unavailable` or `budget_ms` is looser than
    /// declared (`subscription::StaleDifference`).
    pub const PLAN_STALE: &str = errors::PLAN_STALE;
    /// A user turn's `pre_user` hook was unavailable under `refuse`: the run
    /// ends `error` with this `provider_code`, and a steered or queued send
    /// is refused with it and writes nothing.
    pub const PRE_USER_UNAVAILABLE: &str = errors::provider_codes::PRE_USER_UNAVAILABLE;

    /// Reasons the runner writes on a tool call's error result.
    pub mod tool_result_reasons {
        use cortexkit_role_llm_runner::errors::tool_result_reasons as runner;

        /// A `validate` hook denied the call.
        pub const PRE_TOOL_DENIED: &str = runner::PRE_TOOL_DENIED;
        /// A `pre_tool` hook under `refuse` was unavailable.
        pub const PRE_TOOL_UNAVAILABLE: &str = runner::PRE_TOOL_UNAVAILABLE;
        /// A human declined the call at the `approve` phase.
        pub const PRE_TOOL_DECLINED: &str = runner::PRE_TOOL_DECLINED;
        /// The approve-phase question expired under `on_expiry: deny`.
        pub const PRE_TOOL_EXPIRED: &str = runner::PRE_TOOL_EXPIRED;
        /// The call executed, and a `post_tool` hook under `refuse` was
        /// unavailable, so its output was withheld.
        pub const POST_TOOL_UNAVAILABLE: &str = runner::POST_TOOL_UNAVAILABLE;
        pub const ALL: &[&str] = &[
            PRE_TOOL_DENIED,
            PRE_TOOL_UNAVAILABLE,
            PRE_TOOL_DECLINED,
            PRE_TOOL_EXPIRED,
            POST_TOOL_UNAVAILABLE,
        ];
    }
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
        let reasons: Vec<&str> = vectors::cases(&file, "tool_result_reasons")
            .iter()
            .map(|case| case.as_str().unwrap())
            .collect();
        assert_eq!(reasons, runner_codes::tool_result_reasons::ALL);
    }

    #[test]
    fn refused_field_reads_only_invalid_params() {
        let detail = serde_json::json!({"field": "params.tags"});
        assert_eq!(
            refused_field(INVALID_PARAMS, Some(&detail)),
            Some("params.tags")
        );
        assert_eq!(refused_field(NOT_SUBSCRIBED, Some(&detail)), None);
    }
}
