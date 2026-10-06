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
}

// Generate the enumeration and its inventory together: adding a variant must
// also expose it to the contract-parity check. Shared spellings stay tied to
// the runner constants, while serde independently checks snake_case encoding.
macro_rules! known_codes {
    ($($variant:ident => $code:path),+ $(,)?) => {
        /// Named ERROR codes, refusal-answer codes and runner provider codes.
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
    Transient => TRANSIENT,
    WindowTooSmall => refuse_codes::WINDOW_TOO_SMALL,
    ProviderBusy => refuse_codes::PROVIDER_BUSY,
    Misconfigured => refuse_codes::MISCONFIGURED,
    HistoryUnreadable => refuse_codes::HISTORY_UNREADABLE,
    NotSessionCompactionProvider => runner_codes::NOT_SESSION_COMPACTION_PROVIDER,
    CompactionUnavailable => runner_codes::COMPACTION_UNAVAILABLE,
    CompactionWaitExceeded => runner_codes::COMPACTION_WAIT_EXCEEDED,
}

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
    /// A `provider_code` the runner writes itself: Setup failed or timed out
    /// with no answer, so no initial view was recorded and no model call was
    /// made. The next send calls Setup again.
    pub const COMPACTION_UNAVAILABLE: &str = errors::provider_codes::COMPACTION_UNAVAILABLE;
    /// A `provider_code` the runner writes itself: a `wait` reached the
    /// runner's cap and the request could not be shown to fit.
    pub const COMPACTION_WAIT_EXCEEDED: &str = errors::provider_codes::COMPACTION_WAIT_EXCEEDED;
    /// The `provider_code` values the runner writes itself on account of
    /// its compaction provider. A provider's own `refuse` code is written
    /// as given.
    pub const PROVIDER_CODES: &[&str] = &[COMPACTION_UNAVAILABLE, COMPACTION_WAIT_EXCEEDED];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    fn contract_codes() -> std::collections::BTreeSet<String> {
        let contract = include_str!("../CONTRACT.md");
        let refuse = contract
            .split("## 11. REFUSE")
            .nth(1)
            .unwrap()
            .split("## 12.")
            .next()
            .unwrap();
        let errors = contract
            .split("## 14. Error codes")
            .nth(1)
            .unwrap()
            .split("## 15.")
            .next()
            .unwrap();
        [refuse, errors]
            .into_iter()
            .flat_map(|section| {
                section
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
            for detail in [None, Some(serde_json::json!({"field": "preset"}))] {
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
    fn runner_provider_codes_match_the_vectors_and_the_runner_role() {
        let file = vectors::load("errors.json");
        let listed: Vec<&str> = vectors::cases(&file, "runner_provider_codes")
            .iter()
            .map(|case| case.as_str().unwrap())
            .collect();
        assert_eq!(listed, runner_codes::PROVIDER_CODES);
        for code in runner_codes::PROVIDER_CODES {
            assert!(
                cortexkit_role_llm_runner::errors::provider_codes::CODES.contains(code),
                "{code} is not an llm-runner/v1 provider code"
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
