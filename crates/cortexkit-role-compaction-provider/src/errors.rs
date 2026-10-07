//! Codes this role names.
//!
//! Three kinds, kept apart because they reach different places:
//!
//! - a provider's refusal of a request, the `code` of an `ERROR` frame's
//!   body `{code, message, detail?}` ([`INVALID_PARAMS`], [`TRANSIENT`]).
//!   The runner treats any refusal of a step call as a failed call: it
//!   keeps the last applied CompactionMessage;
//! - the `code` of a `refuse` answer, one of the role's fixed codes
//!   ([`RefuseCode`], [`refuse_codes`]); the provider's own, finer reason
//!   rides beside it as the answer's `provider_code`;
//! - codes the runner answers or writes, quoted from `llm-runner/v1` so
//!   both sides spell them the same ([`runner_codes`]).
//!
//! Every named code has a fixed retryability ([`KnownCode::retryable`]:
//! `true` means the runner may retry without the user acting) and a fixed
//! set of calls after which it may end a run ([`KnownCode::ends_run_at`]):
//! the four `refuse` codes after Setup or a step call,
//! `compaction_unavailable` after Setup only, `compaction_wait_exceeded`
//! after a step call only, and the ERROR and ready-refusal codes never.
//!
//! Codes are open strings: a party that meets one it does not know treats
//! it as a terminal refusal of that one request, never retried.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

/// The two calls a runner makes to its compaction provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Call {
    /// `compaction.setup`, once per session before its first model call.
    Setup,
    /// `compaction.step`, the per-step call.
    Step,
}

impl Call {
    pub const ALL: [Call; 2] = [Call::Setup, Call::Step];
}

impl KnownCode {
    /// Whether the code is retryable. Retryability is fixed by the code:
    /// `true` means the runner may retry without the user acting. A
    /// `refuse` answer carries no retryability of its own.
    pub fn retryable(self) -> bool {
        match self {
            Self::Transient
            | Self::ProviderBusy
            | Self::HistoryUnreadable
            | Self::CompactionWaitExceeded => true,
            Self::InvalidParams
            | Self::WindowTooSmall
            | Self::Misconfigured
            | Self::NotSessionCompactionProvider
            | Self::CompactionUnavailable => false,
        }
    }

    /// Whether a run may end `error` with this code because of `call`.
    /// The four `refuse` codes may end it after either call.
    /// `compaction_unavailable` may end it only after Setup: when a step
    /// call fails or times out, the run does not end at all, because the
    /// runner sends the request with the last applied CompactionMessage.
    /// `compaction_wait_exceeded` may end it only after a step call,
    /// because only a step call can be answered `wait`. ERROR codes and
    /// the ready refusal never end a run themselves.
    pub fn ends_run_at(self, call: Call) -> bool {
        match self {
            Self::WindowTooSmall
            | Self::ProviderBusy
            | Self::Misconfigured
            | Self::HistoryUnreadable => true,
            Self::CompactionUnavailable => call == Call::Setup,
            Self::CompactionWaitExceeded => call == Call::Step,
            Self::InvalidParams | Self::Transient | Self::NotSessionCompactionProvider => false,
        }
    }

    /// The code with this spelling, if the role names it.
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|known| known.as_str() == code)
    }
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

/// The `code` of a `refuse` answer: one of the role's four codes, each with
/// a fixed retryability. A code this crate does not know still decodes, as
/// [`RefuseCode::Unknown`], so one unfamiliar code never makes the whole
/// answer undecodable; it is never retryable. A provider's own, finer reason
/// goes in the answer's separate `provider_code`, never in `code`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RefuseCode {
    WindowTooSmall,
    ProviderBusy,
    Misconfigured,
    HistoryUnreadable,
    /// A code this role does not name, kept as received.
    Unknown(String),
}

impl RefuseCode {
    pub fn as_str(&self) -> &str {
        match self {
            Self::WindowTooSmall => refuse_codes::WINDOW_TOO_SMALL,
            Self::ProviderBusy => refuse_codes::PROVIDER_BUSY,
            Self::Misconfigured => refuse_codes::MISCONFIGURED,
            Self::HistoryUnreadable => refuse_codes::HISTORY_UNREADABLE,
            Self::Unknown(code) => code,
        }
    }

    /// The code with this spelling; an unnamed spelling is [`Self::Unknown`].
    pub fn from_code(code: &str) -> Self {
        match code {
            refuse_codes::WINDOW_TOO_SMALL => Self::WindowTooSmall,
            refuse_codes::PROVIDER_BUSY => Self::ProviderBusy,
            refuse_codes::MISCONFIGURED => Self::Misconfigured,
            refuse_codes::HISTORY_UNREADABLE => Self::HistoryUnreadable,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// Whether the refusal is retryable. Retryability is fixed by the code:
    /// `true` means the runner may retry without the user acting. An
    /// unknown code is never retried.
    pub fn retryable(&self) -> bool {
        refuse_codes::retryable(self.as_str())
    }
}

impl Serialize for RefuseCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RefuseCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        Ok(Self::from_code(&code))
    }
}

/// The spellings of the four [`RefuseCode`]s.
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

    /// The retryability the role fixes for a `refuse` code: `false` for a
    /// code it does not name, which is never retried.
    pub fn retryable(code: &str) -> bool {
        matches!(code, PROVIDER_BUSY | HISTORY_UNREADABLE)
    }
}

/// Codes the runner answers or writes, taken from the `llm-runner/v1`
/// crate so both sides spell them the same.
pub mod runner_codes {
    use cortexkit_role_llm_runner::{compaction, errors};

    /// The runner's malformed-request refusal. At admission, the check a
    /// runner makes on a proposed session plan before accepting it, a runner
    /// that does not declare the `compaction` group refuses a plan naming a
    /// compaction item with `detail.field` = [`PLAN_COMPACTION_ITEM`]
    /// (CONTRACT.md §3).
    pub const INVALID_PARAMS: &str = errors::INVALID_PARAMS;
    /// The `detail.field` of that admission refusal: `plan.compaction_item`,
    /// the plan's compaction item, which a runner without the `compaction`
    /// group cannot serve.
    pub const PLAN_COMPACTION_ITEM: &str = compaction::PLAN_COMPACTION_ITEM_FIELD;
    /// A `compaction.ready` whose route caller is not the provider at
    /// `plan.compaction_item.provider`, or for a session without one.
    pub const NOT_SESSION_COMPACTION_PROVIDER: &str = errors::NOT_SESSION_COMPACTION_PROVIDER;
    /// The codes a `compaction.ready` may be refused with.
    pub const READY_CODES: &[&str] = &[INVALID_PARAMS, NOT_SESSION_COMPACTION_PROVIDER];
    /// A `provider_code` the runner writes itself, after Setup only: Setup
    /// failed or timed out with no answer, so no initial view was recorded
    /// and no model call was made. The next send calls Setup again. The
    /// runner never writes this code after a step call: a step call that
    /// fails or times out does not end the run, because the runner sends the
    /// request with the last applied CompactionMessage.
    pub const COMPACTION_UNAVAILABLE: &str = errors::provider_codes::COMPACTION_UNAVAILABLE;
    /// A `provider_code` the runner writes itself: a `wait` reached the
    /// runner's cap and the request could not be shown to fit.
    pub const COMPACTION_WAIT_EXCEEDED: &str = errors::provider_codes::COMPACTION_WAIT_EXCEEDED;
    /// The `provider_code` values the runner writes itself on account of
    /// its compaction provider. A `refuse` answer's code is recorded as
    /// given.
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
            let decoded: RefuseCode = vectors::round_trip(code, &case["code"]);
            assert_eq!(
                matches!(decoded, RefuseCode::Unknown(_)),
                !case["named"].as_bool().unwrap(),
                "{code}"
            );
            assert_eq!(
                decoded.retryable(),
                case["retryable"].as_bool().unwrap(),
                "{code}"
            );
            assert_eq!(refuse_codes::retryable(code), decoded.retryable(), "{code}");
        }
    }

    #[test]
    fn every_refuse_code_has_its_fixed_retryability() {
        let expected = [
            (RefuseCode::WindowTooSmall, false),
            (RefuseCode::ProviderBusy, true),
            (RefuseCode::Misconfigured, false),
            (RefuseCode::HistoryUnreadable, true),
        ];
        assert_eq!(expected.len(), refuse_codes::ALL.len());
        for (code, retryable) in expected {
            assert_eq!(code.retryable(), retryable, "{}", code.as_str());
            assert_eq!(RefuseCode::from_code(code.as_str()), code);
            let known = KnownCode::from_code(code.as_str()).unwrap();
            assert_eq!(known.retryable(), retryable, "{}", code.as_str());
        }
        // A code the role does not name decodes, is kept as received, and
        // is never retried.
        let unknown: RefuseCode = serde_json::from_value(Value::from("acme:quota")).unwrap();
        assert_eq!(unknown, RefuseCode::Unknown("acme:quota".into()));
        assert!(!unknown.retryable());
        assert_eq!(serde_json::to_value(&unknown).unwrap(), "acme:quota");
    }

    /// Reads the retryability table from CONTRACT.md. Each row is one named
    /// code: its spelling, whether it is retryable, and the calls (Setup,
    /// step) after which it may end a run.
    fn contract_retryability() -> Vec<(String, bool, Vec<Call>)> {
        let contract = include_str!("../CONTRACT.md");
        let table = contract
            .split("### 14.3 ")
            .nth(1)
            .expect("CONTRACT.md has no §14.3")
            .split("\n## ")
            .next()
            .unwrap();
        table
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|row| {
                let cells: Vec<&str> = row.split('|').map(str::trim).collect();
                let code = cells[1].trim_matches('`').to_owned();
                let retryable = match cells[2] {
                    "yes" => true,
                    "no" => false,
                    other => panic!("{code}: retryable cell {other:?}"),
                };
                let calls = cells[3]
                    .split(',')
                    .map(str::trim)
                    .filter_map(|call| match call {
                        "Setup" => Some(Call::Setup),
                        "step" => Some(Call::Step),
                        "—" => None,
                        other => panic!("{code}: call cell {other:?}"),
                    })
                    .collect();
                (code, retryable, calls)
            })
            .collect()
    }

    /// Checks that three copies of one table agree: `KnownCode`'s
    /// `retryable` and `ends_run_at`, the retryability table in CONTRACT.md,
    /// and the `retryability` cases in `errors.json`. Each gives, per named
    /// code, whether it is retryable and the calls after which it may end a
    /// run. It also pins that `compaction_unavailable` is never retryable
    /// and ends a run only after Setup.
    #[test]
    fn retryability_table_matches_contract_and_vectors() {
        let table = contract_retryability();
        assert_eq!(table.len(), KnownCode::ALL.len(), "§14.3 rows: {table:?}");
        let file = vectors::load("errors.json");
        let rows = vectors::cases(&file, "retryability");
        assert_eq!(rows.len(), KnownCode::ALL.len());
        for ((code, retryable, calls), row) in table.iter().zip(rows) {
            let known = KnownCode::from_code(code).unwrap_or_else(|| panic!("{code}"));
            assert_eq!(row["code"], code.as_str());
            assert_eq!(known.retryable(), *retryable, "{code}: retryable");
            assert_eq!(row["retryable"], *retryable, "{code}: vector retryable");
            let ends: Vec<Call> = Call::ALL
                .into_iter()
                .filter(|call| known.ends_run_at(*call))
                .collect();
            assert_eq!(&ends, calls, "{code}: ends a run after");
            let vector_calls: Vec<Call> = row["ends_run_at"]
                .as_array()
                .unwrap()
                .iter()
                .map(|call| match call.as_str().unwrap() {
                    "setup" => Call::Setup,
                    "step" => Call::Step,
                    other => panic!("{code}: {other}"),
                })
                .collect();
            assert_eq!(&vector_calls, calls, "{code}: vector ends_run_at");
        }
        let unavailable = KnownCode::CompactionUnavailable;
        assert!(!unavailable.retryable());
        assert!(unavailable.ends_run_at(Call::Setup));
        assert!(!unavailable.ends_run_at(Call::Step));
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
