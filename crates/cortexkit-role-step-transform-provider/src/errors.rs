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

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// An ERROR body. Codes remain open strings, including provider-defined ones.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl ErrorBody {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Recovery hint on a transient history refusal. The provider supplies
    /// the first missing ordinal; another refusal code gives it no meaning.
    pub fn history_gap_from(&self) -> Option<u64> {
        if self.code != TRANSIENT {
            return None;
        }
        self.detail.as_ref()?.get("history_gap_from")?.as_u64()
    }
}

// Generate the enumeration and its inventory together so a new variant cannot
// evade the contract-parity check by being left out of a hand-written list.
macro_rules! known_codes {
    ($($variant:ident => $code:path),+ $(,)?) => {
        /// Named ERROR codes, runner provider codes and tool-result reasons.
        /// This classification enum is not the open wire type of `code`.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
        #[serde(rename_all = "snake_case")]
        pub enum KnownCode { $($variant),+ }

        impl KnownCode {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $code),+ }
            }
        }
    };
}

known_codes! {
    InvalidParams => INVALID_PARAMS,
    NotSubscribed => NOT_SUBSCRIBED,
    Transient => TRANSIENT,
    PlanStale => runner_codes::PLAN_STALE,
    PreUserUnavailable => runner_codes::PRE_USER_UNAVAILABLE,
    PreToolDenied => runner_codes::tool_result_reasons::PRE_TOOL_DENIED,
    PreToolUnavailable => runner_codes::tool_result_reasons::PRE_TOOL_UNAVAILABLE,
    PreToolDeclined => runner_codes::tool_result_reasons::PRE_TOOL_DECLINED,
    PreToolExpired => runner_codes::tool_result_reasons::PRE_TOOL_EXPIRED,
    PostToolUnavailable => runner_codes::tool_result_reasons::POST_TOOL_UNAVAILABLE,
}

/// A request field is malformed, misconfigured or conflicts with an already
/// ingested subject (an unknown serializer profile or changed message, say).
/// `detail.field` names it. The malformed-request code of the runner and
/// provider roles alike; `tool-provider/v1` spells it `invalid_request`.
pub const INVALID_PARAMS: &str = "invalid_params";

/// A hook call for a hook and phase the provider did not declare for the
/// call's preset and params. `detail` carries `hook` and, on `pre_tool`,
/// `phase`.
pub const NOT_SUBSCRIBED: &str = "not_subscribed";

/// The provider could not answer now for a reason that may clear by itself.
/// Retryable, but only within the hook's budget.
/// For a history gap, `detail.history_gap_from` may name the first missing
/// ordinal. Other codes ignore that hint.
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

    /// The runner's refusal of a malformed session plan. Admission is the
    /// plan check before the runner accepts the session (CONTRACT.md §4).
    /// It refuses malformed step-transform subscriptions and a `replace` on
    /// `pre_user` or `post_assistant` assigned to anyone but the reduction
    /// owner: the session's compaction provider, which alone may remove or
    /// rewrite history (CONTRACT.md §5). This prevents another transform
    /// from erasing content it may only prepend or append to. The detail
    /// names [`PLAN_STEP_TRANSFORM_ITEMS`] as its `field` and identifies the
    /// invalid subscription (`subscription::InvalidSubscriptionDetail`).
    pub const INVALID_PARAMS: &str = errors::INVALID_PARAMS;
    /// The `detail.field` value that locates an invalid subscription in the
    /// proposed plan's step-transform list. The detail is `{field:
    /// "plan.step_transform_items", item, subscription, problem}`: `item` is
    /// the index of the plan item, `subscription` the index of the
    /// subscription within it, and `problem` the name of what is wrong (for
    /// example `ops_on_pre_tool` or `replace_not_reduction_owner`), so the
    /// plan composer can find and correct that subscription (CONTRACT.md §4).
    pub const PLAN_STEP_TRANSFORM_ITEMS: &str = "plan.step_transform_items";
    /// The runner refuses this at admission, the plan check before accepting
    /// a session, when a provider's current declaration no longer covers the
    /// plan: a hook or preset is gone, or planned tools, operations,
    /// availability policy or budget exceed the declared bounds. The detail
    /// is `{differences: [...]}`, one entry per subscription that no longer
    /// fits, each tagged by `kind`: `subscription_missing {provider, hook,
    /// phase}` when the declaration lacks the hook and phase,
    /// `subscription_loosened {provider, hook, phase, field}` when the plan's
    /// `tools`, `ops`, `on_unavailable` or `budget_ms` is looser than
    /// declared, and `preset_missing {provider, preset}`, once per item, when
    /// the provider no longer knows the preset (`subscription::StaleDifference`).
    /// The plan composer must rebuild the plan from the current declaration
    /// (CONTRACT.md §4).
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

    fn contract_codes() -> std::collections::BTreeSet<String> {
        let contract = include_str!("../CONTRACT.md");
        let errors = contract
            .split("## 9. Error codes")
            .nth(1)
            .unwrap()
            .split("## 10.")
            .next()
            .unwrap();
        errors
            .lines()
            .filter_map(|line| line.strip_prefix("| "))
            .flat_map(|row| {
                row.split('|')
                    .next()
                    .unwrap()
                    .split('`')
                    .enumerate()
                    .filter(|(index, _)| index % 2 == 1)
                    .map(|(_, value)| value.split_whitespace().next().unwrap().to_owned())
            })
            .collect()
    }

    #[test]
    fn refusal_codes_match_contract() {
        let documented = contract_codes();
        assert!(!documented.is_empty(), "no refusal-code tables found");
        let encoded: std::collections::BTreeSet<String> = KnownCode::ALL
            .iter()
            .map(|code| {
                let wire = serde_json::to_value(code).unwrap();
                assert_eq!(wire.as_str().unwrap(), code.as_str());
                wire.as_str().unwrap().to_owned()
            })
            .collect();
        assert_eq!(
            encoded.len(),
            KnownCode::ALL.len(),
            "duplicate code variants"
        );
        assert_eq!(
            documented, encoded,
            "CONTRACT.md and Rust refusal variants differ"
        );
        for name in documented {
            let code: KnownCode = vectors::round_trip(&name, &Value::String(name.clone()));
            assert_eq!(code.as_str(), name);
        }
    }

    #[test]
    fn every_refusal_body_round_trips() {
        for code in contract_codes().into_iter().chain(["acme:busy".to_owned()]) {
            for detail in [
                None,
                Some(serde_json::json!({"hook": "pre_tool", "phase": "validate"})),
            ] {
                let mut expected = serde_json::json!({"code": code, "message": "unavailable"});
                if let Some(value) = detail {
                    expected["detail"] = value;
                }
                vectors::round_trip::<ErrorBody>(&code, &expected);
            }
        }
    }

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
