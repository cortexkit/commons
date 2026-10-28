//! Codes this role names.
//!
//! Three kinds, kept apart because they reach different places:
//!
//! - a provider's refusal of a request, the `code` of an `ERROR` frame's
//!   body `{code, message, detail?}` ([`INVALID_PARAMS`], [`TRANSIENT`]);
//! - the `code` of a Setup `refuse` answer, one of the role's fixed codes
//!   ([`RefuseCode`], [`refuse_codes`]); the provider's own, finer reason
//!   rides beside it as the answer's `provider_code`;
//! - codes the runner answers or writes, quoted from `llm-runner/v1` so
//!   both sides spell them the same ([`runner_codes`]).
//!
//! Every code in `KnownCode` has a fixed retryability ([`KnownCode::retryable`]:
//! `true` means the runner may retry without the user acting) and a fixed
//! answer to whether it may end a run after Setup
//! ([`KnownCode::ends_run_after_setup`]): the typed `refuse` codes and
//! `compaction_unavailable` may, the ERROR codes never. A named wire code not
//! in `KnownCode` follows `RefuseCode::Unknown` when it refuses Setup.
//! Both enums are `#[non_exhaustive]`: a later code is additive, so a
//! `match` outside this crate keeps a `_` arm.
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
        ///
        /// Non-exhaustive: a later code is additive, so a `match` outside
        /// this crate keeps a `_` arm.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
        #[serde(rename_all = "snake_case")]
        #[non_exhaustive]
        pub enum KnownCode { $($variant),+ }

        impl KnownCode {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $code),+ }
            }
        }
    };
}

impl KnownCode {
    /// Whether the code is retryable. Retryability is fixed by the code:
    /// `true` means the runner may retry without the user acting. A
    /// `refuse` answer carries no retryability of its own.
    pub fn retryable(self) -> bool {
        match self {
            Self::Transient | Self::ProviderBusy | Self::HistoryUnreadable => true,
            Self::InvalidParams
            | Self::WindowTooSmall
            | Self::Misconfigured
            | Self::StandaloneSessionNotMigrated
            | Self::TagCheckpointUnsupported
            | Self::MessageCheckpointUnsupported
            | Self::MovedSessionRevertedBelowImport
            | Self::MovedSessionHeadMismatch
            | Self::SetupIdentityMismatch
            | Self::CompactionUnavailable => false,
        }
    }

    /// Whether a run may end `error` with this code after Setup: the typed
    /// `refuse` codes, and `compaction_unavailable` when Setup failed or
    /// timed out with no answer. ERROR codes never end a run themselves.
    /// `standalone_session_not_migrated` ends the run except in its one
    /// hand-off: on a host running standalone Magic Context in-process, a
    /// runner that has never served the session hands it to standalone in
    /// the same step instead (CONTRACT.md §4).
    pub fn ends_run_after_setup(self) -> bool {
        match self {
            Self::WindowTooSmall
            | Self::ProviderBusy
            | Self::Misconfigured
            | Self::HistoryUnreadable
            | Self::TagCheckpointUnsupported
            | Self::MessageCheckpointUnsupported
            | Self::MovedSessionRevertedBelowImport
            | Self::MovedSessionHeadMismatch
            | Self::SetupIdentityMismatch
            | Self::StandaloneSessionNotMigrated
            | Self::CompactionUnavailable => true,
            Self::InvalidParams | Self::Transient => false,
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
    StandaloneSessionNotMigrated => refuse_codes::STANDALONE_SESSION_NOT_MIGRATED,
    TagCheckpointUnsupported => refuse_codes::TAG_CHECKPOINT_UNSUPPORTED,
    MessageCheckpointUnsupported => refuse_codes::MESSAGE_CHECKPOINT_UNSUPPORTED,
    MovedSessionRevertedBelowImport => refuse_codes::MOVED_SESSION_REVERTED_BELOW_IMPORT,
    MovedSessionHeadMismatch => refuse_codes::MOVED_SESSION_HEAD_MISMATCH,
    SetupIdentityMismatch => refuse_codes::SETUP_IDENTITY_MISMATCH,
    CompactionUnavailable => runner_codes::COMPACTION_UNAVAILABLE,
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

/// The `code` of a Setup `refuse` answer: one of the typed codes, each
/// with a fixed retryability. A code this crate does not know still decodes,
/// as [`RefuseCode::Unknown`], so one unfamiliar code never makes the whole
/// answer undecodable; an unfamiliar code is never retryable. A provider's own, finer reason
/// goes in the answer's separate `provider_code`, never in `code`.
///
/// Non-exhaustive: a later code is additive, so a `match` outside this crate
/// keeps a `_` arm.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RefuseCode {
    WindowTooSmall,
    ProviderBusy,
    Misconfigured,
    HistoryUnreadable,
    /// The session has standalone Magic Context state and was never moved
    /// to the provider. A runner that has never served the session, on a
    /// host running standalone in-process, hands it to standalone; any other
    /// runner fails the run by name.
    StandaloneSessionNotMigrated,
    /// The session was moved from standalone Magic Context and needs its
    /// Setup checkpoint honoured, which the runner does not declare.
    TagCheckpointUnsupported,
    /// A moved session needs message checkpoints, which the runner lacks.
    MessageCheckpointUnsupported,
    /// The moved session has no head cut and its first ordinal is zero.
    MovedSessionRevertedBelowImport,
    /// The head otherwise disagrees with the certified move checkpoint.
    MovedSessionHeadMismatch,
    /// The incoming route is bound to a non-empty session other than
    /// Setup's session. Not retryable; it ends the current run.
    SetupIdentityMismatch,
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
            Self::StandaloneSessionNotMigrated => refuse_codes::STANDALONE_SESSION_NOT_MIGRATED,
            Self::TagCheckpointUnsupported => refuse_codes::TAG_CHECKPOINT_UNSUPPORTED,
            Self::MessageCheckpointUnsupported => refuse_codes::MESSAGE_CHECKPOINT_UNSUPPORTED,
            Self::MovedSessionRevertedBelowImport => {
                refuse_codes::MOVED_SESSION_REVERTED_BELOW_IMPORT
            }
            Self::MovedSessionHeadMismatch => refuse_codes::MOVED_SESSION_HEAD_MISMATCH,
            Self::SetupIdentityMismatch => refuse_codes::SETUP_IDENTITY_MISMATCH,
            Self::Unknown(code) => code,
        }
    }

    /// Maps a `refuse` code string to one of the typed variants; a wire
    /// code this crate does not list becomes [`Self::Unknown`].
    pub fn from_code(code: &str) -> Self {
        match code {
            refuse_codes::WINDOW_TOO_SMALL => Self::WindowTooSmall,
            refuse_codes::PROVIDER_BUSY => Self::ProviderBusy,
            refuse_codes::MISCONFIGURED => Self::Misconfigured,
            refuse_codes::HISTORY_UNREADABLE => Self::HistoryUnreadable,
            refuse_codes::STANDALONE_SESSION_NOT_MIGRATED => Self::StandaloneSessionNotMigrated,
            refuse_codes::TAG_CHECKPOINT_UNSUPPORTED => Self::TagCheckpointUnsupported,
            refuse_codes::MESSAGE_CHECKPOINT_UNSUPPORTED => Self::MessageCheckpointUnsupported,
            refuse_codes::MOVED_SESSION_REVERTED_BELOW_IMPORT => {
                Self::MovedSessionRevertedBelowImport
            }
            refuse_codes::MOVED_SESSION_HEAD_MISMATCH => Self::MovedSessionHeadMismatch,
            refuse_codes::SETUP_IDENTITY_MISMATCH => Self::SetupIdentityMismatch,
            other => Self::Unknown(other.to_owned()),
        }
    }

    /// Whether this value is the `setup_identity_mismatch` refusal: its
    /// typed variant, or the same spelling kept as [`Self::Unknown`] by a
    /// value built by hand.
    pub fn is_setup_identity_mismatch(&self) -> bool {
        self.as_str() == refuse_codes::SETUP_IDENTITY_MISMATCH
    }

    /// Whether the refusal is retryable. Retryability is fixed by the code:
    /// `true` means the runner may retry without the user acting. An unknown
    /// code is never retried.
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

/// A Setup whose `descends_from` names a cut this provider never published
/// is refused with this code, never answered with a guess. It can happen
/// only when a runner cuts on its own. It is not one of the typed
/// [`RefuseCode`]s: it decodes as [`RefuseCode::Unknown`], which a runner
/// never retries, so it is terminal for that lineage.
pub const UNKNOWN_CUT: &str = "unknown_cut";

/// The spellings of the typed [`RefuseCode`]s. On any of them the
/// runner ends the current run as `error` after Setup, but for the one
/// hand-off of `standalone_session_not_migrated` (CONTRACT.md §4).
pub mod refuse_codes {
    /// The model's window cannot hold the smallest view the provider can
    /// build. The user has to switch to a larger model. Not retryable.
    pub const WINDOW_TOO_SMALL: &str = "window_too_small";
    /// The provider is busy with the session's history and cannot answer
    /// Setup now. Retryable: the next send may succeed.
    pub const PROVIDER_BUSY: &str = "provider_busy";
    /// The provider's configuration for this preset and params cannot be
    /// used. The user has to fix it. Not retryable.
    pub const MISCONFIGURED: &str = "misconfigured";
    /// The provider needed history it could not read, such as a declared
    /// inherited prefix it does not hold. Retryable.
    pub const HISTORY_UNREADABLE: &str = "history_unreadable";
    /// The session has standalone Magic Context state and no completed move
    /// to this provider. Terminal: not retryable. On a host running
    /// standalone in-process, a runner that has never served the session
    /// hands it to standalone in the same step and sends no rides for it;
    /// once a runner has served a session it never hands it back, and this
    /// refusal fails the run by name. A host without standalone (a
    /// module-launched private host, the Claude Code gateway) fails the run,
    /// or the request, by name and never forwards unmanaged history.
    pub const STANDALONE_SESSION_NOT_MIGRATED: &str = "standalone_session_not_migrated";
    /// The session was moved from standalone Magic Context, so its numbering
    /// needs the Setup checkpoint, and the runner does not declare
    /// `llm-runner/v1`'s `tag_checkpoint` group. Terminal: not retryable.
    pub const TAG_CHECKPOINT_UNSUPPORTED: &str = "tag_checkpoint_unsupported";
    /// A moved session's runner lacks `message_checkpoint`. Terminal.
    pub const MESSAGE_CHECKPOINT_UNSUPPORTED: &str = "message_checkpoint_unsupported";
    /// The moved session's head has no cut and first_ordinal is 0. Terminal.
    pub const MOVED_SESSION_REVERTED_BELOW_IMPORT: &str = "moved_session_reverted_below_import";
    /// The head otherwise mismatches the certified count or head cut. Terminal.
    pub const MOVED_SESSION_HEAD_MISMATCH: &str = "moved_session_head_mismatch";

    /// A route bound to a non-empty session different from Setup's session.
    /// Not retryable: the runner ends the current run as `error` and calls
    /// Setup again only on the next send.
    pub const SETUP_IDENTITY_MISMATCH: &str = "setup_identity_mismatch";

    pub const ALL: &[&str] = &[
        WINDOW_TOO_SMALL,
        PROVIDER_BUSY,
        MISCONFIGURED,
        HISTORY_UNREADABLE,
        STANDALONE_SESSION_NOT_MIGRATED,
        TAG_CHECKPOINT_UNSUPPORTED,
        MESSAGE_CHECKPOINT_UNSUPPORTED,
        MOVED_SESSION_REVERTED_BELOW_IMPORT,
        MOVED_SESSION_HEAD_MISMATCH,
        SETUP_IDENTITY_MISMATCH,
    ];

    /// Whether this `refuse` code may be retried without the user acting. Only
    /// `provider_busy` and `history_unreadable` are; any other code, including
    /// one this crate does not list, is never retried.
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
    /// A `provider_code` the runner writes itself, after Setup only: Setup
    /// failed or timed out with no answer, so no initial view was recorded
    /// and no model call was made. The next send calls Setup again.
    pub const COMPACTION_UNAVAILABLE: &str = errors::provider_codes::COMPACTION_UNAVAILABLE;
    /// The `provider_code` values the runner writes itself on account of
    /// its compaction provider. A `refuse` answer's code is recorded as
    /// given.
    pub const PROVIDER_CODES: &[&str] = &[COMPACTION_UNAVAILABLE];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    fn contract_codes() -> std::collections::BTreeSet<String> {
        let contract = include_str!("../CONTRACT.md");
        let refuse = contract
            .split("## 10. REFUSE")
            .nth(1)
            .unwrap()
            .split("## 11.")
            .next()
            .unwrap();
        let errors = contract
            .split("## 13. Error codes")
            .nth(1)
            .unwrap()
            .split("## 14.")
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
    fn every_typed_refuse_code_has_its_fixed_retryability() {
        let expected = [
            (RefuseCode::WindowTooSmall, false),
            (RefuseCode::ProviderBusy, true),
            (RefuseCode::Misconfigured, false),
            (RefuseCode::HistoryUnreadable, true),
            (RefuseCode::StandaloneSessionNotMigrated, false),
            (RefuseCode::TagCheckpointUnsupported, false),
            (RefuseCode::MessageCheckpointUnsupported, false),
            (RefuseCode::MovedSessionRevertedBelowImport, false),
            (RefuseCode::MovedSessionHeadMismatch, false),
            (RefuseCode::SetupIdentityMismatch, false),
        ];
        assert_eq!(expected.len(), refuse_codes::ALL.len());
        for (code, retryable) in expected {
            assert_eq!(code.retryable(), retryable, "{}", code.as_str());
            assert_eq!(RefuseCode::from_code(code.as_str()), code);
            let known = KnownCode::from_code(code.as_str()).unwrap();
            assert_eq!(known.retryable(), retryable, "{}", code.as_str());
        }
        // A code not listed in `KnownCode` still decodes, is kept as received,
        // and is never retried.
        let unknown: RefuseCode = serde_json::from_value(Value::from("acme:quota")).unwrap();
        assert_eq!(unknown, RefuseCode::Unknown("acme:quota".into()));
        assert!(!unknown.retryable());
        assert_eq!(serde_json::to_value(&unknown).unwrap(), "acme:quota");
    }

    /// `setup_identity_mismatch` is a typed refusal: it decodes to its own
    /// variant, is never retried, ends the run after Setup, and round-trips
    /// as the same spelling.
    #[test]
    fn setup_identity_mismatch_is_a_typed_refusal() {
        let code = RefuseCode::from_code(refuse_codes::SETUP_IDENTITY_MISMATCH);
        assert_eq!(code, RefuseCode::SetupIdentityMismatch);
        assert!(code.is_setup_identity_mismatch());
        assert!(!code.retryable());
        let known = KnownCode::from_code(code.as_str()).unwrap();
        assert_eq!(known, KnownCode::SetupIdentityMismatch);
        assert!(!known.retryable());
        assert!(known.ends_run_after_setup());
        assert!(include_str!("../CONTRACT.md").contains("| `setup_identity_mismatch` |"));

        let answer: crate::setup::SetupAnswer = serde_json::from_value(serde_json::json!({
            "answer": "refuse",
            "request_id": "request-1",
            "code": refuse_codes::SETUP_IDENTITY_MISMATCH,
            "reason": "the route is bound to another session"
        }))
        .unwrap();
        assert_eq!(answer.retryable(), Some(false));
        match &answer {
            crate::setup::SetupAnswer::Refuse { code, .. } => {
                assert_eq!(code, &RefuseCode::SetupIdentityMismatch);
            }
            crate::setup::SetupAnswer::Ready { .. } => panic!("decoded a refusal as ready"),
        }
        assert_eq!(
            serde_json::to_value(&answer).unwrap()["code"],
            refuse_codes::SETUP_IDENTITY_MISMATCH
        );
    }

    /// Reads the retryability table from CONTRACT.md. Each row is one named
    /// code: its spelling, whether it is retryable, and whether it may end a
    /// run after Setup.
    fn contract_retryability() -> Vec<(String, bool, bool)> {
        let contract = include_str!("../CONTRACT.md");
        let table = contract
            .split("### 13.3 ")
            .nth(1)
            .expect("CONTRACT.md has no §13.3")
            .split("\n## ")
            .next()
            .unwrap();
        let yes_no = |code: &str, cell: &str| match cell {
            "yes" => true,
            "no" => false,
            other => panic!("{code}: cell {other:?}"),
        };
        table
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|row| {
                let cells: Vec<&str> = row.split('|').map(str::trim).collect();
                let code = cells[1].trim_matches('`').to_owned();
                let retryable = yes_no(&code, cells[2]);
                let ends_run = yes_no(&code, cells[3]);
                (code, retryable, ends_run)
            })
            .collect()
    }

    #[test]
    fn retryability_table_matches_contract_and_vectors() {
        let table = contract_retryability();
        let file = vectors::load("errors.json");
        let rows = vectors::cases(&file, "retryability");
        assert_eq!(rows.len(), table.len());
        let mut typed_count = 0;
        for ((code, retryable, ends_run), row) in table.iter().zip(rows) {
            assert_eq!(row["code"], code.as_str());
            assert_eq!(row["retryable"], *retryable, "{code}: vector retryable");
            assert_eq!(
                row["ends_run_after_setup"], *ends_run,
                "{code}: vector ends run"
            );

            typed_count += 1;
            let known = KnownCode::from_code(code).unwrap_or_else(|| panic!("{code}"));
            assert_eq!(known.retryable(), *retryable, "{code}: retryable");
            assert_eq!(
                known.ends_run_after_setup(),
                *ends_run,
                "{code}: ends a run"
            );
        }
        assert_eq!(
            typed_count,
            KnownCode::ALL.len(),
            "§13.3 typed rows: {table:?}"
        );
        let unavailable = KnownCode::CompactionUnavailable;
        assert!(!unavailable.retryable());
        assert!(unavailable.ends_run_after_setup());
        // A session that was never moved ends the run but for the hand-off
        // to an in-process standalone; a moved session on a runner that
        // cannot honour its checkpoint ends the run.
        assert!(KnownCode::StandaloneSessionNotMigrated.ends_run_after_setup());
        assert!(KnownCode::TagCheckpointUnsupported.ends_run_after_setup());
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
