//! Refusal codes and the refusal body.
//!
//! A participant refuses a request with an ERROR body `{code, message,
//! detail?}` ([`ErrorBody`]). `code` is one of the closed set in
//! CONTRACT.md §6 ([`RefusalCode`]); the op and code fix what the driver does
//! next ([`driver_action_for`]). A participant cannot invent codes: one outside
//! the set still decodes, as [`RefusalCode::Unknown`], so an unfamiliar
//! code never makes the whole body undecodable, and the driver never retries
//! it: it aborts the move and shows the code.
//!
//! Five codes require a typed `detail` ([`Detail`]); `digest_mismatch` may
//! carry an optional [`DigestMismatchDetail`], decoded separately so the
//! existing exhaustive `Detail` enum stays source-compatible.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

use crate::{refs::Ref, step::Step, Op};

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
    /// The agent's data is in use (a turn or run in flight, a queued send,
    /// an open approval card, a background job): on the source's `prepare`
    /// it would change during the move; on the receiver's `import` a run
    /// may have started on the same session since `diverged` (the receiver
    /// isn't frozen); on the receiver's `activate` the staged data can't go
    /// live yet. `detail.refs` names the refs and why. Only those three ops
    /// may refuse it ([`super::allowed_refusals`]).
    pub const BUSY: &str = "busy";
    /// On the source, the data must never leave this Mac by its own rules
    /// (for example, broca sessions whose caller chose a retention that
    /// keeps them local).
    pub const NOT_EXPORTABLE: &str = "not_exportable";
    /// On the receiver (`diverged` or `import`), this participant's store
    /// can't accept this cut: the cut's store version (an item's `format`)
    /// is one it cannot read, so the receiving Mac must be upgraded first.
    pub const INCOMPATIBLE_VERSION: &str = "incompatible_version";
    /// Bytes received or held by this participant don't hash to their recorded
    /// digest: the incoming cut on `import`, or its own staged bytes or undo
    /// images on `activate` and `abort`. See [`super::driver_action_for`].
    pub const DIGEST_MISMATCH: &str = "digest_mismatch";
    /// On the receiver, this Mac's copy of these refs gained data after
    /// `diverged` answered `at_cut`, so `import` or `activate` would
    /// overwrite it. Terminal for the step and never retried
    /// ([`super::DriverAction::TerminalForStep`]): after `import` the driver
    /// aborts the move and offers the operator the fork choice; after
    /// `activate` the move holds as a fork for the operator. The
    /// participant leaves its staged import intact and its local copy
    /// untouched, so whichever side the operator picks can still be applied.
    pub const LOCAL_ADVANCED: &str = "local_advanced";
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
        LOCAL_ADVANCED,
        EXTERNAL_NOT_PRESENT,
        UNAVAILABLE,
    ];
}

/// What the driver (entorhinal) does after a refusal. The op and refusal code
/// determine its action ([`driver_action_for`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DriverAction {
    /// Stop the move. Never retried.
    Abort,
    /// Send the same request again, within the step it was refused in.
    RetryWithinStep,
    /// Send the same request again later; the move waits.
    RetryLater,
    /// Send the same request again after the operator has copied the host's
    /// own data to this Mac, or repaired the participant's undo state for
    /// `digest_mismatch` on `abort`.
    RetryAfterOperator,
    /// Send the same request again, with no condition attached (a
    /// temporary failure).
    Retry,
    /// The step ends here and the request is never sent again; what the
    /// driver does next depends on the step (CONTRACT.md §7.7, §7.8). After
    /// `import` it aborts the move and offers the operator the fork choice;
    /// after `activate` it cannot abort, so the move holds as a fork until
    /// the operator resolves it.
    TerminalForStep,
}

impl DriverAction {
    /// Whether the driver sends the same request again. [`Abort`] and
    /// [`TerminalForStep`] are never retried.
    ///
    /// [`Abort`]: DriverAction::Abort
    /// [`TerminalForStep`]: DriverAction::TerminalForStep
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::RetryWithinStep | Self::RetryLater | Self::RetryAfterOperator | Self::Retry
        )
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
    LocalAdvanced,
    ExternalNotPresent,
    Unavailable,
    /// A code outside the closed set. Never retried: the driver aborts the
    /// move and shows the code.
    Unknown(String),
}

impl RefusalCode {
    /// Every code in the closed set, in the order of CONTRACT.md §6.
    pub const KNOWN: [RefusalCode; 14] = [
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
        Self::LocalAdvanced,
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
            Self::LocalAdvanced => codes::LOCAL_ADVANCED,
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

    /// The code-only driver action, retained for existing callers. Use
    /// [`driver_action_for`] when the op is known: `digest_mismatch` needs a
    /// different action on `activate` and `abort` (CONTRACT.md §6).
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
            Self::LocalAdvanced => DriverAction::TerminalForStep,
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
/// ([`ErrorBody::typed_detail`], [`ErrorBody::digest_mismatch_detail`]).
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

    /// A `digest_mismatch` refusal with an optional reason identifying whether
    /// the corrupted bytes belong to staging or undo images.
    pub fn with_digest_mismatch_detail(
        detail: DigestMismatchDetail,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: RefusalCode::DigestMismatch,
            message: message.into(),
            detail: Some(serde_json::to_value(detail).expect("detail is JSON")),
        }
    }

    /// The optional reason detail for [`codes::DIGEST_MISMATCH`]. `Ok(None)` for another
    /// code or an absent detail; an error if a present detail does not decode.
    pub fn digest_mismatch_detail(
        &self,
    ) -> Result<Option<DigestMismatchDetail>, serde_json::Error> {
        if self.code != RefusalCode::DigestMismatch {
            return Ok(None);
        }
        self.detail
            .as_ref()
            .map(|value| {
                if !value.is_object() {
                    return Err(serde::de::Error::custom(
                        "digest_mismatch detail must be an object",
                    ));
                }
                serde_json::from_value(value.clone())
            })
            .transpose()
    }

    /// The required typed detail for the five codes represented by [`Detail`].
    /// `Ok(None)` for other codes, including `digest_mismatch` (use
    /// [`Self::digest_mismatch_detail`]); an error when a required detail is
    /// absent or does not decode.
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

    /// Adds the refs the `cutId` started with: the set the participant
    /// expected the refused request to name.
    pub fn with_refs(mut self, refs: Vec<Ref>) -> Self {
        self.refs = Some(refs);
        self
    }
}

/// `busy`: `{refs: [{ref_kind, ref, reason_code?, reason}]}`, the refs that
/// are in use (on `prepare`, can't be cut now; on `activate`, can't go live
/// now), each with a reason.
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

/// Known reasons for `digest_mismatch`. The wire remains an open string so a
/// participant can describe a more specific digest failure without a new code.
pub mod digest_mismatch_reason {
    /// This participant's staged bytes no longer match their recorded digest.
    pub const STAGING_DIGEST_MISMATCH: &str = "staging_digest_mismatch";
    /// This participant's undo images no longer match their recorded digest.
    pub const UNDO_DIGEST_MISMATCH: &str = "undo_digest_mismatch";
}

/// Optional `digest_mismatch` detail. Kept outside [`Detail`] to preserve
/// existing callers' exhaustive matches on its five required-detail variants.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DigestMismatchDetail {
    /// An open reason string; known spellings are in [`digest_mismatch_reason`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl DigestMismatchDetail {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
        }
    }
}

/// Select the driver action for an op and code (CONTRACT.md §6). A digest
/// failure on `activate` is terminal for the step and holds as a fork for the
/// operator. It cannot return the claim: other participants may already be
/// live. On `abort`, retry only after the operator repairs undo state; the
/// move is not yet aborted and the source agent remains frozen until success.
/// All other pairs retain [`RefusalCode::driver_action`].
pub fn driver_action_for(op: Op, code: &RefusalCode) -> DriverAction {
    match (op, code) {
        (Op::Activate, RefusalCode::DigestMismatch) => DriverAction::TerminalForStep,
        (Op::Abort, RefusalCode::DigestMismatch) => DriverAction::RetryAfterOperator,
        _ => code.driver_action(),
    }
}

/// The codes each op may refuse with: the table in CONTRACT.md §6.3. Every
/// op may refuse `caller_forbidden`, `invalid_request` and `unavailable`;
/// every op that changes state may also refuse `id_reused`. A participant
/// that refuses an op with a code outside its row breaks the contract, and
/// a code outside the closed set is never in any row.
pub fn allowed_refusals(op: Op) -> &'static [RefusalCode] {
    use RefusalCode as C;
    match op {
        Op::Describe | Op::Status => &[C::CallerForbidden, C::InvalidRequest, C::Unavailable],
        Op::Busy => &[
            C::CallerForbidden,
            C::InvalidRequest,
            C::Unavailable,
            C::UnknownRef,
        ],
        Op::Diverged => &[
            C::CallerForbidden,
            C::InvalidRequest,
            C::Unavailable,
            C::ExternalNotPresent,
            C::CutNotReady,
            C::IncompatibleVersion,
        ],
        Op::Prepare => &[
            C::CallerForbidden,
            C::InvalidRequest,
            C::Unavailable,
            C::IdReused,
            C::UnknownRef,
            C::InvalidState,
            C::Busy,
            C::NotExportable,
        ],
        Op::Export | Op::Release => &[
            C::CallerForbidden,
            C::InvalidRequest,
            C::Unavailable,
            C::IdReused,
            C::UnknownRef,
            C::InvalidState,
            C::CutUnknown,
        ],
        Op::Import => &[
            C::CallerForbidden,
            C::InvalidRequest,
            C::Unavailable,
            C::IdReused,
            C::InvalidState,
            C::CutNotReady,
            C::IncompatibleVersion,
            C::DigestMismatch,
            C::Busy,
            C::LocalAdvanced,
        ],
        Op::Activate => &[
            C::CallerForbidden,
            C::InvalidRequest,
            C::Unavailable,
            C::IdReused,
            C::InvalidState,
            C::CutUnknown,
            C::ExternalNotPresent,
            C::Busy,
            C::LocalAdvanced,
            C::DigestMismatch,
        ],
        Op::Abort => &[
            C::CallerForbidden,
            C::InvalidRequest,
            C::Unavailable,
            C::IdReused,
            C::InvalidState,
            C::DigestMismatch,
        ],
    }
}

/// Whether `op` may refuse with `code` ([`allowed_refusals`]).
pub fn may_refuse(op: Op, code: &RefusalCode) -> bool {
    allowed_refusals(op).contains(code)
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

    /// The rows of the refusal table in CONTRACT.md §6: each code and its
    /// driver column, whose opening words name the `DriverAction` the test
    /// compares against.
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
        } else if cell.starts_with("Terminal for the step") {
            DriverAction::TerminalForStep
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

    /// Reads the per-op table in CONTRACT.md §6.3 (each op's name and the
    /// codes in its row), so a test can check it against
    /// `allowed_refusals`.
    fn contract_op_table() -> Vec<(String, Vec<String>)> {
        let contract = include_str!("../CONTRACT.md");
        let section = contract
            .split("### 6.3 ")
            .nth(1)
            .expect("CONTRACT.md has no §6.3")
            .split("\n## ")
            .next()
            .unwrap();
        section
            .lines()
            .filter(|line| {
                line.starts_with("| ") && !line.starts_with("| Op") && !line.starts_with("| ---")
            })
            .map(|row| {
                let cells: Vec<&str> = row.split('|').map(str::trim).collect();
                let codes = cells[2]
                    .split(',')
                    .map(|code| code.trim().trim_matches('`').to_owned())
                    .collect();
                (cells[1].to_owned(), codes)
            })
            .collect()
    }

    #[test]
    fn per_op_refusals_match_contract() {
        let table = contract_op_table();
        assert_eq!(table.len(), Op::ALL.len(), "{table:?}");
        for ((name, codes), op) in table.iter().zip(Op::ALL) {
            assert_eq!(format!("agent_data.{name}"), op.method());
            let typed: Vec<&str> = allowed_refusals(op).iter().map(|c| c.as_str()).collect();
            assert_eq!(&typed, codes, "{name}");
        }
    }

    #[test]
    fn driver_action_for_preserves_legacy_actions_except_digest_overrides() {
        let mut codes = RefusalCode::KNOWN.to_vec();
        codes.push(RefusalCode::Unknown("future_code".into()));
        let mut checked = 0;
        for op in Op::ALL {
            for code in &codes {
                let expected = match (op, code) {
                    (Op::Activate, RefusalCode::DigestMismatch) => DriverAction::TerminalForStep,
                    (Op::Abort, RefusalCode::DigestMismatch) => DriverAction::RetryAfterOperator,
                    _ => code.driver_action(),
                };
                assert_eq!(driver_action_for(op, code), expected, "{op:?} / {code}");
                checked += 1;
            }
        }
        assert_eq!(checked, 150, "all ten ops and fourteen codes plus unknown");
        assert_eq!(
            RefusalCode::DigestMismatch.driver_action(),
            DriverAction::Abort
        );
        assert!(!RefusalCode::DigestMismatch.retryable());
    }

    #[test]
    fn digest_mismatch_is_allowed_only_on_import_activate_and_abort() {
        for op in Op::ALL {
            assert_eq!(
                may_refuse(op, &RefusalCode::DigestMismatch),
                matches!(op, Op::Import | Op::Activate | Op::Abort),
                "{op:?}"
            );
        }
    }

    #[test]
    fn digest_mismatch_contract_actions_are_per_op() {
        let table = contract_table();
        let (_, driver) = table
            .iter()
            .find(|(code, _)| code == codes::DIGEST_MISMATCH)
            .unwrap();
        let cells: Vec<_> = driver.split("; ").collect();
        assert_eq!(cells.len(), 3);
        for (cell, op) in cells.iter().zip([Op::Import, Op::Activate, Op::Abort]) {
            let name = op.method().strip_prefix("agent_data.").unwrap();
            assert!(cell.contains(&format!("`{name}`")), "{cell}");
            assert_eq!(
                driver_action_for(op, &RefusalCode::DigestMismatch),
                action_from_contract(cell),
                "{op:?}"
            );
        }
    }

    #[test]
    fn digest_mismatch_detail_is_optional_and_reasons_are_open() {
        let mut body = ErrorBody::new(RefusalCode::DigestMismatch, "digest failed");
        assert_eq!(body.digest_mismatch_detail().unwrap(), None);
        body.detail = Some(serde_json::json!({}));
        assert_eq!(
            body.digest_mismatch_detail().unwrap(),
            Some(DigestMismatchDetail::default())
        );
        for reason in [
            digest_mismatch_reason::STAGING_DIGEST_MISMATCH,
            digest_mismatch_reason::UNDO_DIGEST_MISMATCH,
            "future_digest_reason",
        ] {
            let body = ErrorBody::with_digest_mismatch_detail(
                DigestMismatchDetail::new(reason),
                "digest failed",
            );
            assert_eq!(body.detail, Some(serde_json::json!({"reason": reason})));
            assert_eq!(
                body.digest_mismatch_detail()
                    .unwrap()
                    .unwrap()
                    .reason
                    .as_deref(),
                Some(reason)
            );
            assert_eq!(body.typed_detail().unwrap(), None);
        }
        let body = ErrorBody::new(RefusalCode::Unavailable, "later");
        assert_eq!(body.digest_mismatch_detail().unwrap(), None);
        for code in [
            RefusalCode::UnknownRef,
            RefusalCode::InvalidState,
            RefusalCode::Busy,
            RefusalCode::IncompatibleVersion,
            RefusalCode::ExternalNotPresent,
        ] {
            assert!(ErrorBody::new(code, "missing detail")
                .typed_detail()
                .is_err());
        }
    }

    #[test]
    fn digest_mismatch_details_reject_malformed_shapes() {
        for detail in [
            serde_json::json!({"reason": 1}),
            serde_json::json!({"reason": "undo_digest_mismatch", "extra": true}),
            serde_json::json!(null),
            serde_json::json!([]),
        ] {
            let mut body = ErrorBody::new(RefusalCode::DigestMismatch, "digest failed");
            body.detail = Some(detail);
            assert!(body.digest_mismatch_detail().is_err(), "{:?}", body.detail);
        }
    }

    #[test]
    fn local_advanced_is_terminal_for_the_step() {
        let code = RefusalCode::LocalAdvanced;
        assert_eq!(code.as_str(), "local_advanced");
        assert_eq!(RefusalCode::from_code("local_advanced"), code);
        assert_eq!(code.driver_action(), DriverAction::TerminalForStep);
        assert!(!code.retryable());
        let ops: Vec<Op> = Op::ALL
            .into_iter()
            .filter(|op| may_refuse(*op, &code))
            .collect();
        assert_eq!(ops, [Op::Import, Op::Activate]);
    }

    #[test]
    fn diverged_may_refuse_incompatible_version() {
        assert!(may_refuse(Op::Diverged, &RefusalCode::IncompatibleVersion));
        assert!(may_refuse(Op::Import, &RefusalCode::IncompatibleVersion));
        assert!(!RefusalCode::IncompatibleVersion.retryable());
    }

    #[test]
    fn diverged_may_refuse_cut_not_ready() {
        assert!(may_refuse(Op::Diverged, &RefusalCode::CutNotReady));
        assert!(may_refuse(Op::Import, &RefusalCode::CutNotReady));
        assert_eq!(
            RefusalCode::CutNotReady.driver_action(),
            DriverAction::RetryWithinStep
        );
    }

    #[test]
    fn busy_is_refused_only_on_prepare_import_and_activate() {
        let ops: Vec<Op> = Op::ALL
            .into_iter()
            .filter(|op| may_refuse(*op, &RefusalCode::Busy))
            .collect();
        assert_eq!(ops, [Op::Prepare, Op::Import, Op::Activate]);
        assert!(!may_refuse(
            Op::Activate,
            &RefusalCode::Unknown("busy_ish".into())
        ));
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
