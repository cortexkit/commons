//! Refusal codes and the refusal body.
//!
//! A participant refuses a request with an ERROR body `{code, message,
//! detail?}` ([`ErrorBody`]). `code` is one of the closed set in
//! CONTRACT.md §6 ([`RefusalCode`]); each code fixes what the driver does
//! next ([`DriverAction`]). A participant cannot invent codes: one outside
//! the set still decodes, as [`RefusalCode::Unknown`], so an unfamiliar
//! code never makes the whole body undecodable, and the driver never retries
//! it: it aborts the move and shows the code.
//!
//! Five codes carry a typed `detail` ([`Detail`]); the others carry none.

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::{refs::Ref, step::Step};

/// The spellings of the closed refusal set: the thirteen codes in the table
/// of CONTRACT.md §6, in its order. No participant may answer with any
/// other code; one that does gets the move aborted.
pub mod codes {
    /// The caller the daemon stamped on the route is not
    /// `reserved:entorhinal`.
    pub const CALLER_FORBIDDEN: &str = "caller_forbidden";
    /// The request does not decode: malformed, a field missing or of the
    /// wrong type, an unknown field where the contract types one, more than
    /// 256 refs, a ref listed twice, or a malformed `cutId`.
    pub const INVALID_REQUEST: &str = "invalid_request";
    /// On the source, the participant answering doesn't hold one of the
    /// request's refs; `detail` names the first one.
    pub const UNKNOWN_REF: &str = "unknown_ref";
    /// The participant already answered this `(cutId, op)` and the new
    /// request differs from the one it answered.
    pub const ID_REUSED: &str = "id_reused";
    /// The participant's own state for this `cutId` doesn't allow the op:
    /// its step is wrong (for example `release` before `export`), or the op
    /// names another ref set than the `cutId` started with (`prepare`'s on
    /// the source, `import`'s on the receiver). `detail.step` names the
    /// step; `detail.refs` names the expected refs in the second case.
    pub const INVALID_STATE: &str = "invalid_state";
    /// The participant answering has no state for this `cutId` (for
    /// example, `export` before `prepare`).
    pub const CUT_UNKNOWN: &str = "cut_unknown";
    /// On the receiver, engram's session door can't serve this cut's bytes
    /// yet (`cut.read` or `session.read` isn't ready for the `cutId`).
    pub const CUT_NOT_READY: &str = "cut_not_ready";
    /// On the source, the agent's data is in use and would change during
    /// the move (a turn or run in flight, a queued send, an open approval
    /// card, a background job); `detail.refs` names the refs and why.
    pub const BUSY: &str = "busy";
    /// On the source, the data must never leave this Mac by its own rules
    /// (for example, broca sessions whose caller chose a retention that
    /// keeps them local).
    pub const NOT_EXPORTABLE: &str = "not_exportable";
    /// On the receiver, this participant's store can't accept this cut:
    /// the cut's store version (an item's `format`) is one it cannot read.
    pub const INCOMPATIBLE_VERSION: &str = "incompatible_version";
    /// On the receiver, the bytes read through the door don't hash to the
    /// digest the source gave in `prepare`.
    pub const DIGEST_MISMATCH: &str = "digest_mismatch";
    /// On the receiver, the host program's own data for a ref (external
    /// data) isn't on this Mac yet, or doesn't match the `check` the source
    /// gave in `prepare`.
    pub const EXTERNAL_NOT_PRESENT: &str = "external_not_present";
    /// The participant answering failed for a reason that may clear by
    /// itself (its store is locked, it is restarting).
    pub const UNAVAILABLE: &str = "unavailable";

    /// Every code in the closed set.
    pub const ALL: &[&str] = &[
        CALLER_FORBIDDEN,
        INVALID_REQUEST,
        UNKNOWN_REF,
        ID_REUSED,
        INVALID_STATE,
        CUT_UNKNOWN,
        CUT_NOT_READY,
        BUSY,
        NOT_EXPORTABLE,
        INCOMPATIBLE_VERSION,
        DIGEST_MISMATCH,
        EXTERNAL_NOT_PRESENT,
        UNAVAILABLE,
    ];
}

/// What the driver (entorhinal) does after a refusal. Each refusal `code`
/// determines its action ([`RefusalCode::driver_action`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DriverAction {
    /// Stop the move. Never retried.
    Abort,
    /// Send the same request again, within the step it was refused in.
    RetryWithinStep,
    /// Send the same request again later; the move waits.
    RetryLater,
    /// Send the same request again after the operator has copied the host's
    /// own data to this Mac.
    RetryAfterOperator,
    /// Send the same request again, with no condition attached (a
    /// temporary failure).
    Retry,
}

impl DriverAction {
    /// Whether the driver sends the same request again. Only
    /// [`DriverAction::Abort`] is not retried.
    pub fn retryable(self) -> bool {
        self != Self::Abort
    }
}

/// The `code` of a refusal: the closed set of CONTRACT.md §6, plus
/// [`RefusalCode::Unknown`] for a code outside it, kept as received.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RefusalCode {
    CallerForbidden,
    InvalidRequest,
    UnknownRef,
    IdReused,
    InvalidState,
    CutUnknown,
    CutNotReady,
    Busy,
    NotExportable,
    IncompatibleVersion,
    DigestMismatch,
    ExternalNotPresent,
    Unavailable,
    /// A code outside the closed set. Never retried: the driver aborts the
    /// move and shows the code.
    Unknown(String),
}

impl RefusalCode {
    /// Every code in the closed set, in the order of CONTRACT.md §6.
    pub const KNOWN: [RefusalCode; 13] = [
        Self::CallerForbidden,
        Self::InvalidRequest,
        Self::UnknownRef,
        Self::IdReused,
        Self::InvalidState,
        Self::CutUnknown,
        Self::CutNotReady,
        Self::Busy,
        Self::NotExportable,
        Self::IncompatibleVersion,
        Self::DigestMismatch,
        Self::ExternalNotPresent,
        Self::Unavailable,
    ];

    pub fn as_str(&self) -> &str {
        match self {
            Self::CallerForbidden => codes::CALLER_FORBIDDEN,
            Self::InvalidRequest => codes::INVALID_REQUEST,
            Self::UnknownRef => codes::UNKNOWN_REF,
            Self::IdReused => codes::ID_REUSED,
            Self::InvalidState => codes::INVALID_STATE,
            Self::CutUnknown => codes::CUT_UNKNOWN,
            Self::CutNotReady => codes::CUT_NOT_READY,
            Self::Busy => codes::BUSY,
            Self::NotExportable => codes::NOT_EXPORTABLE,
            Self::IncompatibleVersion => codes::INCOMPATIBLE_VERSION,
            Self::DigestMismatch => codes::DIGEST_MISMATCH,
            Self::ExternalNotPresent => codes::EXTERNAL_NOT_PRESENT,
            Self::Unavailable => codes::UNAVAILABLE,
            Self::Unknown(code) => code,
        }
    }

    /// The code with this spelling; a spelling outside the closed set
    /// becomes [`RefusalCode::Unknown`].
    pub fn from_code(code: &str) -> Self {
        Self::KNOWN
            .into_iter()
            .find(|known| known.as_str() == code)
            .unwrap_or_else(|| Self::Unknown(code.to_owned()))
    }

    /// What the driver does after this refusal (CONTRACT.md §6).
    pub fn driver_action(&self) -> DriverAction {
        match self {
            Self::CutNotReady => DriverAction::RetryWithinStep,
            Self::Busy => DriverAction::RetryLater,
            Self::ExternalNotPresent => DriverAction::RetryAfterOperator,
            Self::Unavailable => DriverAction::Retry,
            Self::CallerForbidden
            | Self::InvalidRequest
            | Self::UnknownRef
            | Self::IdReused
            | Self::InvalidState
            | Self::CutUnknown
            | Self::NotExportable
            | Self::IncompatibleVersion
            | Self::DigestMismatch
            | Self::Unknown(_) => DriverAction::Abort,
        }
    }

    /// Whether the driver sends the same request again after this refusal.
    /// A code outside the closed set never is.
    pub fn retryable(&self) -> bool {
        self.driver_action().retryable()
    }

    /// Whether this code is in the closed set.
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

impl std::fmt::Display for RefusalCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl Serialize for RefusalCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RefusalCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let code = String::deserialize(deserializer)?;
        Ok(Self::from_code(&code))
    }
}

/// A refusal: the daemon's ERROR body, `{code, message, detail?}`, the
/// shape every role uses. Decoded leniently (unknown members are ignored),
/// like the other roles' ERROR bodies, because the daemon owns this frame's
/// shape; the `detail` objects CONTRACT.md §6.2 defines decode strictly
/// ([`ErrorBody::typed_detail`]).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ErrorBody {
    pub code: RefusalCode,
    /// Short operator text. Never parsed.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl ErrorBody {
    pub fn new(code: RefusalCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            detail: None,
        }
    }

    /// A refusal carrying a typed detail; the code is the one the detail
    /// belongs to.
    pub fn with_typed_detail(detail: Detail, message: impl Into<String>) -> Self {
        let (code, value) = detail.into_parts();
        Self {
            code,
            message: message.into(),
            detail: Some(value),
        }
    }

    /// The typed detail for this body's code. `Ok(None)` for a code that
    /// carries none; an error when a code that carries one has none, or it
    /// does not decode.
    pub fn typed_detail(&self) -> Result<Option<Detail>, serde_json::Error> {
        fn take<T: DeserializeOwned>(detail: &Option<Value>) -> Result<T, serde_json::Error> {
            match detail {
                Some(value) => serde_json::from_value(value.clone()),
                None => Err(serde::de::Error::custom("this refusal code needs a detail")),
            }
        }
        Ok(Some(match self.code {
            RefusalCode::InvalidState => Detail::InvalidState(take(&self.detail)?),
            RefusalCode::Busy => Detail::Busy(take(&self.detail)?),
            RefusalCode::UnknownRef => Detail::UnknownRef(take(&self.detail)?),
            RefusalCode::IncompatibleVersion => Detail::IncompatibleVersion(take(&self.detail)?),
            RefusalCode::ExternalNotPresent => Detail::ExternalNotPresent(take(&self.detail)?),
            _ => return Ok(None),
        }))
    }
}

/// The typed `detail` of the five codes that carry one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detail {
    InvalidState(InvalidStateDetail),
    Busy(BusyDetail),
    UnknownRef(UnknownRefDetail),
    IncompatibleVersion(IncompatibleVersionDetail),
    ExternalNotPresent(ExternalNotPresentDetail),
}

impl Detail {
    /// The code this detail belongs to, and the detail as JSON.
    pub fn into_parts(self) -> (RefusalCode, Value) {
        fn json<T: Serialize>(detail: T) -> Value {
            serde_json::to_value(detail).expect("a detail always serializes")
        }
        match self {
            Self::InvalidState(d) => (RefusalCode::InvalidState, json(d)),
            Self::Busy(d) => (RefusalCode::Busy, json(d)),
            Self::UnknownRef(d) => (RefusalCode::UnknownRef, json(d)),
            Self::IncompatibleVersion(d) => (RefusalCode::IncompatibleVersion, json(d)),
            Self::ExternalNotPresent(d) => (RefusalCode::ExternalNotPresent, json(d)),
        }
    }
}

/// `invalid_state`: `{step, refs?}`. `step` is the participant's current
/// step for the `cutId`. `refs` is present only when the step allowed the
/// op but the request named another ref set than the `cutId` started with
/// ([`crate::step::check_ref_set`]): it lists the refs the participant
/// expects, in the order the first op named them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct InvalidStateDetail {
    pub step: Step,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<Vec<Ref>>,
}

impl InvalidStateDetail {
    pub fn new(step: Step) -> Self {
        Self { step, refs: None }
    }

    /// Adds the refs the participant expects.
    pub fn with_refs(mut self, refs: Vec<Ref>) -> Self {
        self.refs = Some(refs);
        self
    }
}

/// `busy`: `{refs: [{ref_kind, ref, reason_code?, reason}]}`, the refs that
/// can't be cut now, each with a reason.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BusyDetail {
    pub refs: Vec<BusyRef>,
}

impl BusyDetail {
    pub fn new(refs: Vec<BusyRef>) -> Self {
        Self { refs }
    }
}

/// One busy ref in a `busy` refusal.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BusyRef {
    pub ref_kind: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// The same short code the `busy` op answers with, which the operator's
    /// phone shows without parsing text ([`crate::busy::reason_codes`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
    /// Short operator text.
    pub reason: String,
}

impl BusyRef {
    pub fn new(
        ref_kind: impl Into<String>,
        r#ref: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        Self {
            ref_kind: ref_kind.into(),
            r#ref: r#ref.into(),
            reason_code: None,
            reason: reason.into(),
        }
    }

    pub fn with_reason_code(mut self, reason_code: impl Into<String>) -> Self {
        self.reason_code = Some(reason_code.into());
        self
    }
}

/// `unknown_ref`: `{ref_kind, ref}`, the first ref of the request this
/// participant doesn't hold.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct UnknownRefDetail {
    pub ref_kind: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

impl UnknownRefDetail {
    pub fn new(ref_kind: impl Into<String>, r#ref: impl Into<String>) -> Self {
        Self {
            ref_kind: ref_kind.into(),
            r#ref: r#ref.into(),
        }
    }
}

/// `incompatible_version`: `{found, required}`, the version the cut
/// carries and the one this participant's store needs. Both opaque text.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct IncompatibleVersionDetail {
    pub found: String,
    pub required: String,
}

impl IncompatibleVersionDetail {
    pub fn new(found: impl Into<String>, required: impl Into<String>) -> Self {
        Self {
            found: found.into(),
            required: required.into(),
        }
    }
}

/// `external_not_present`: `{note}`, repeating the operator note `prepare`
/// gave for the host's data.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExternalNotPresentDetail {
    pub note: String,
}

impl ExternalNotPresentDetail {
    pub fn new(note: impl Into<String>) -> Self {
        Self { note: note.into() }
    }
}

/// Decodes a request's params as `T`. Anything that does not decode (an
/// unknown top-level or typed field, more than 256 refs, a malformed
/// `cutId`, a missing or mistyped field) is refused `invalid_request`, with
/// the decoder's message.
pub fn decode_request<T: DeserializeOwned>(params: &Value) -> Result<T, ErrorBody> {
    T::deserialize(params)
        .map_err(|error| ErrorBody::new(RefusalCode::InvalidRequest, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of the refusal table in CONTRACT.md §6: each code and the
    /// first words of its driver column.
    fn contract_table() -> Vec<(String, String)> {
        let contract = include_str!("../CONTRACT.md");
        let section = contract
            .split("## 6. Refusal codes")
            .nth(1)
            .expect("CONTRACT.md has no §6")
            .split("\n## ")
            .next()
            .unwrap();
        section
            .lines()
            .filter(|line| line.starts_with("| `"))
            .map(|row| {
                let cells: Vec<&str> = row.split('|').map(str::trim).collect();
                (cells[1].trim_matches('`').to_owned(), cells[3].to_owned())
            })
            .collect()
    }

    fn action_from_contract(cell: &str) -> DriverAction {
        if cell.starts_with("Abort") {
            DriverAction::Abort
        } else if cell.starts_with("Retry within the step") {
            DriverAction::RetryWithinStep
        } else if cell.starts_with("Retry later") {
            DriverAction::RetryLater
        } else if cell.starts_with("Retry after the operator") {
            DriverAction::RetryAfterOperator
        } else if cell == "Retry" {
            DriverAction::Retry
        } else {
            panic!("unrecognised driver cell {cell:?}")
        }
    }

    #[test]
    fn refusal_table_matches_contract() {
        let table = contract_table();
        assert_eq!(table.len(), RefusalCode::KNOWN.len(), "{table:?}");
        for ((code, driver), known) in table.iter().zip(RefusalCode::KNOWN) {
            assert_eq!(code, known.as_str());
            assert_eq!(
                known.driver_action(),
                action_from_contract(driver),
                "{code}"
            );
            assert_eq!(RefusalCode::from_code(code), known);
            assert_eq!(serde_json::to_value(&known).unwrap(), code.as_str());
        }
        let listed: Vec<&str> = RefusalCode::KNOWN.iter().map(|c| c.as_str()).collect();
        assert_eq!(listed, codes::ALL);
    }

    #[test]
    fn unknown_code_decodes_and_aborts() {
        let code: RefusalCode = serde_json::from_value(Value::from("acme_new_code")).unwrap();
        assert_eq!(code, RefusalCode::Unknown("acme_new_code".into()));
        assert_eq!(code.driver_action(), DriverAction::Abort);
        assert!(!code.retryable());
        assert!(!code.is_known());
        assert_eq!(serde_json::to_value(&code).unwrap(), "acme_new_code");
    }

    #[test]
    fn retryable_codes_are_exactly_four() {
        let retryable: Vec<&str> = RefusalCode::KNOWN
            .iter()
            .filter(|code| code.retryable())
            .map(|code| code.as_str())
            .collect();
        assert_eq!(
            retryable,
            [
                codes::CUT_NOT_READY,
                codes::BUSY,
                codes::EXTERNAL_NOT_PRESENT,
                codes::UNAVAILABLE
            ]
        );
    }

    #[test]
    fn detail_is_required_for_the_codes_that_carry_one() {
        let body = ErrorBody::new(RefusalCode::InvalidState, "out of order");
        assert!(body.typed_detail().is_err());
        let body = ErrorBody::new(RefusalCode::Unavailable, "later");
        assert_eq!(body.typed_detail().unwrap(), None);
    }

    #[test]
    fn typed_details_are_strict() {
        let mut body = ErrorBody::with_typed_detail(
            Detail::InvalidState(InvalidStateDetail::new(Step::Prepared)),
            "out of order",
        );
        assert!(body.typed_detail().is_ok());
        body.detail.as_mut().unwrap()["extra"] = Value::from(1);
        assert!(body.typed_detail().is_err());
    }
}
