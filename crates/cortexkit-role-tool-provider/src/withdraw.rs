//! `tool.withdraw`: withdrawing a call the provider holds.
//!
//! A provider that can hold a call past its own reply (an approval-gated call
//! waiting for its answer, say) serves `tool.withdraw`. The request is an
//! ordinary tool request on the provider's tool route, named `tool.withdraw`;
//! it is not a model tool. The call to withdraw is `arguments.call_key`, and
//! the provider's record for it is keyed `(carrier, call_key)`, where the
//! carrier is the principal that raised the call.
//!
//! Who may withdraw, and the route error each failed check gets (all four
//! are terminal for the caller, and none is an answer):
//! - `arguments.scope`, when present, must equal the route's stamped scope:
//!   otherwise `withdraw_scope_mismatch`.
//! - A caller other than the scope's owner is the carrier: the record is keyed
//!   on the route's stamped principal, and an `arguments.carrier` naming
//!   anyone else is `withdraw_carrier_mismatch`.
//! - The scope's owner may withdraw any carrier's call by naming
//!   `arguments.carrier`. An owner that names no carrier is treated as the
//!   carrier itself; if it holds no call under that key, the request is
//!   `withdraw_carrier_required`.
//! - On an unscoped route only the carrier may withdraw, keyed on the route's
//!   principal.
//! - A record found under a different scope than the route's stamp (or a
//!   scoped record reached from an unscoped route, or the reverse) is
//!   `withdraw_not_permitted`.
//!
//! The answer is read from the provider's record, so it is identical for
//! every permitted caller and every repeat. `unknown_call` means the provider
//! holds no record under that `(carrier, call_key)`; it is never a refusal and
//! never means "not sent".

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

use crate::{
    call::{validate_call_key, ToolCallRequest, CALL_KEY_FIELD},
    errors, ops,
    scope::ScopeIdentity,
};

/// The `field` an `invalid_request` names when the target key is malformed.
pub const TARGET_CALL_KEY_FIELD: &str = "arguments.call_key";

/// Answer names.
pub mod answers {
    pub const WITHDRAWN: &str = "withdrawn";
    pub const ALREADY_STARTED: &str = "already_started";
    pub const COMPLETED: &str = "completed";
    pub const REFUSED: &str = "refused";
    pub const UNKNOWN_CALL: &str = "unknown_call";
}

/// Refusal reasons the role names. Any other string may appear on the wire;
/// it decodes as [`RefusalReason::Other`].
pub mod reasons {
    pub const DENIED: &str = "denied";
    pub const EXPIRED: &str = "expired";
    pub const SCOPE_ENDED: &str = "scope_ended";
}

/// The outcome an `already_started` answer carries when the provider cannot
/// tell whether the call settled. Never replaced by a guessed success.
pub const OUTCOME_UNKNOWN: &str = "unknown";

/// The arguments of a `tool.withdraw` request.
///
/// Non-exhaustive so later optional members are additive: use
/// [`WithdrawArguments::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct WithdrawArguments {
    /// The key of the call to withdraw.
    pub call_key: String,
    /// The principal that carried the call, for example `reserved:broca`.
    /// Required when the caller is the scope's owner rather than the carrier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub carrier: Option<String>,
    /// The scope the caller believes the call is under. Checked against the
    /// route's stamp; never authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeIdentity>,
}

impl WithdrawArguments {
    /// Withdraw the named call with no optional attribution checks.
    pub fn new(call_key: impl Into<String>) -> Self {
        Self {
            call_key: call_key.into(),
            carrier: None,
            scope: None,
        }
    }

    /// Name the principal that carried the call.
    pub fn with_carrier(mut self, carrier: impl Into<String>) -> Self {
        self.carrier = Some(carrier.into());
        self
    }

    /// Set the scope to check against the route's stamp.
    pub fn with_scope(mut self, scope: ScopeIdentity) -> Self {
        self.scope = Some(scope);
        self
    }

    /// The request that carries these arguments. It has no top-level
    /// `call_key`: that would name the withdraw request itself, which is not
    /// a call anyone can hold.
    pub fn into_request(self) -> ToolCallRequest {
        ToolCallRequest::new(
            ops::TOOL_WITHDRAW,
            serde_json::to_value(self).expect("withdraw arguments serialize"),
        )
    }
}

/// The provider's first check of a `tool.withdraw` request: a top-level
/// `call_key` is refused as `invalid_request {field: "call_key"}`, arguments
/// that do not decode as `invalid_request {field: "arguments"}`, and a
/// malformed target key as `invalid_request {field: "arguments.call_key"}`.
pub fn parse_withdraw_request(request: &ToolCallRequest) -> Result<WithdrawArguments, ErrorBody> {
    if request.call_key.is_some() {
        return Err(errors::invalid_request(
            CALL_KEY_FIELD,
            "tool.withdraw names its target in arguments.call_key and carries no call_key of its own",
        ));
    }
    let arguments: WithdrawArguments = serde_json::from_value(request.arguments.clone())
        .map_err(|error| errors::invalid_request("arguments", error.to_string()))?;
    validate_call_key(&arguments.call_key)
        .map_err(|error| errors::invalid_request(TARGET_CALL_KEY_FIELD, error.to_string()))?;
    Ok(arguments)
}

/// Why a withdraw caller's request is refused. Each maps to one route error
/// ([`CallerProblem::error`]); none is an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallerProblem {
    /// `arguments.scope` differs from the route's stamped scope (or names a
    /// scope on an unscoped route).
    ScopeMismatch,
    /// The scope's owner named no carrier and holds no call of its own under
    /// the key.
    CarrierRequired,
    /// A caller other than the scope's owner named a carrier other than
    /// itself.
    CarrierMismatch { named: String },
    /// The record found is under a different scope than the route's stamp.
    NotPermitted,
}

impl CallerProblem {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ScopeMismatch => errors::WITHDRAW_SCOPE_MISMATCH,
            Self::CarrierRequired => errors::WITHDRAW_CARRIER_REQUIRED,
            Self::CarrierMismatch { .. } => errors::WITHDRAW_CARRIER_MISMATCH,
            Self::NotPermitted => errors::WITHDRAW_NOT_PERMITTED,
        }
    }

    /// The route error a provider answers with.
    pub fn error(&self) -> ErrorBody {
        let message = match self {
            Self::ScopeMismatch => "arguments.scope differs from the route's scope".to_owned(),
            Self::CarrierRequired => {
                "the scope's owner must name arguments.carrier for a call it did not raise"
                    .to_owned()
            }
            Self::CarrierMismatch { named } => {
                format!("only the scope's owner may name another carrier ({named})")
            }
            Self::NotPermitted => "the call is under another scope".to_owned(),
        };
        ErrorBody::new(self.code(), message)
    }
}

/// Whose record a withdraw request addresses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CarrierResolution {
    /// Read the record `(carrier, arguments.call_key)`.
    Carrier(String),
    /// The scope's owner named no carrier: read `(owner, arguments.call_key)`,
    /// treating the owner as the carrier. If no such record exists, refuse
    /// with [`CallerProblem::CarrierRequired`] rather than answering
    /// `unknown_call`, because the owner may simply have omitted the carrier.
    OwnerAsCarrier(String),
}

/// Decide whose record a withdraw request addresses, from the route's
/// stamped principal (`caller`) and scope (`stamp_scope`, `None` on an
/// unscoped route) and the request's arguments.
pub fn resolve_carrier(
    caller: &str,
    stamp_scope: Option<&ScopeIdentity>,
    arguments: &WithdrawArguments,
) -> Result<CarrierResolution, CallerProblem> {
    if let Some(named) = &arguments.scope {
        if stamp_scope != Some(named) {
            return Err(CallerProblem::ScopeMismatch);
        }
    }
    let is_owner = stamp_scope.is_some_and(|scope| scope.owner == caller);
    match (&arguments.carrier, is_owner) {
        (Some(named), true) => Ok(CarrierResolution::Carrier(named.clone())),
        (None, true) => Ok(CarrierResolution::OwnerAsCarrier(caller.to_owned())),
        (Some(named), false) if named != caller => Err(CallerProblem::CarrierMismatch {
            named: named.clone(),
        }),
        (_, false) => Ok(CarrierResolution::Carrier(caller.to_owned())),
    }
}

/// The last caller check, once a record was found: the record's scope must
/// be the route's stamped scope (both `None` on unscoped routes).
pub fn check_record_scope(
    record_scope: Option<&ScopeIdentity>,
    stamp_scope: Option<&ScopeIdentity>,
) -> Result<(), CallerProblem> {
    if record_scope == stamp_scope {
        Ok(())
    } else {
        Err(CallerProblem::NotPermitted)
    }
}

/// What `already_started` says about the call's outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartedOutcome {
    /// The provider observed this outcome code.
    Known(String),
    /// The provider cannot tell whether the call settled.
    Unknown,
}

/// What `completed` carries.
#[derive(Clone, Debug, PartialEq)]
pub enum CompletedResult {
    /// `result_retained: true`: the recorded reply.
    Retained(Value),
    /// `result_retained: false`: only the outcome code was kept.
    OutcomeOnly(String),
}

/// Why a withdraw was refused. The reason is an open string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefusalReason {
    Denied,
    Expired,
    ScopeEnded,
    Other(String),
}

impl RefusalReason {
    fn parse(reason: &str) -> Self {
        match reason {
            reasons::DENIED => Self::Denied,
            reasons::EXPIRED => Self::Expired,
            reasons::SCOPE_ENDED => Self::ScopeEnded,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Denied => reasons::DENIED,
            Self::Expired => reasons::EXPIRED,
            Self::ScopeEnded => reasons::SCOPE_ENDED,
            Self::Other(reason) => reason,
        }
    }
}

/// A decoded `tool.withdraw` answer.
///
/// Every answer is final: a caller that gets one records it and stops
/// retrying. That holds for [`WithdrawAnswer::Unclassified`] too, an
/// `answer` this crate does not know: the caller records the whole reply
/// body verbatim.
#[derive(Clone, Debug, PartialEq)]
pub enum WithdrawAnswer {
    /// The call is guaranteed never to run.
    Withdrawn,
    /// The call had started; `outcome` is what the provider observed.
    AlreadyStarted { outcome: StartedOutcome },
    /// The call had completed.
    Completed { result: CompletedResult },
    /// The provider refused the withdrawal.
    Refused { reason: RefusalReason },
    /// The provider holds no record under that `(carrier, call_key)`.
    UnknownCall,
    /// An `answer` this crate does not know: final, but unclassified.
    Unclassified { answer: String, body: Value },
}

/// Why a withdraw reply is not an answer at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WithdrawDecodeError {
    NotAnObject,
    /// The reply has no string `answer`.
    MissingAnswer,
    /// A known answer lacks a member it must carry, or carries it with the
    /// wrong type.
    BadMember {
        answer: &'static str,
        member: &'static str,
    },
}

impl std::fmt::Display for WithdrawDecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnObject => f.write_str("withdraw reply is not an object"),
            Self::MissingAnswer => f.write_str("withdraw reply has no string `answer`"),
            Self::BadMember { answer, member } => {
                write!(f, "`{answer}` answer lacks a well-typed `{member}`")
            }
        }
    }
}

impl std::error::Error for WithdrawDecodeError {}

impl WithdrawAnswer {
    /// Decode a withdraw reply body. `answer`, `outcome` and `reason` are
    /// open strings.
    pub fn decode(body: &Value) -> Result<Self, WithdrawDecodeError> {
        let object = body.as_object().ok_or(WithdrawDecodeError::NotAnObject)?;
        let answer = object
            .get("answer")
            .and_then(Value::as_str)
            .ok_or(WithdrawDecodeError::MissingAnswer)?;
        Ok(match answer {
            answers::WITHDRAWN => Self::Withdrawn,
            answers::UNKNOWN_CALL => Self::UnknownCall,
            answers::ALREADY_STARTED => {
                let outcome = member_str(object, answers::ALREADY_STARTED, "outcome")?;
                Self::AlreadyStarted {
                    outcome: if outcome == OUTCOME_UNKNOWN {
                        StartedOutcome::Unknown
                    } else {
                        StartedOutcome::Known(outcome.to_owned())
                    },
                }
            }
            answers::COMPLETED => {
                let retained = object
                    .get("result_retained")
                    .and_then(Value::as_bool)
                    .ok_or(WithdrawDecodeError::BadMember {
                        answer: answers::COMPLETED,
                        member: "result_retained",
                    })?;
                let result = if retained {
                    CompletedResult::Retained(object.get("result").cloned().ok_or(
                        WithdrawDecodeError::BadMember {
                            answer: answers::COMPLETED,
                            member: "result",
                        },
                    )?)
                } else {
                    CompletedResult::OutcomeOnly(
                        member_str(object, answers::COMPLETED, "outcome")?.to_owned(),
                    )
                };
                Self::Completed { result }
            }
            answers::REFUSED => Self::Refused {
                reason: RefusalReason::parse(member_str(object, answers::REFUSED, "reason")?),
            },
            other => Self::Unclassified {
                answer: other.to_owned(),
                body: body.clone(),
            },
        })
    }

    /// The reply body for this answer.
    pub fn encode(&self) -> Value {
        match self {
            Self::Withdrawn => json!({ "answer": answers::WITHDRAWN }),
            Self::UnknownCall => json!({ "answer": answers::UNKNOWN_CALL }),
            Self::AlreadyStarted { outcome } => json!({
                "answer": answers::ALREADY_STARTED,
                "outcome": match outcome {
                    StartedOutcome::Known(code) => code.as_str(),
                    StartedOutcome::Unknown => OUTCOME_UNKNOWN,
                },
            }),
            Self::Completed {
                result: CompletedResult::Retained(result),
            } => json!({
                "answer": answers::COMPLETED,
                "result": result,
                "result_retained": true,
            }),
            Self::Completed {
                result: CompletedResult::OutcomeOnly(outcome),
            } => json!({
                "answer": answers::COMPLETED,
                "outcome": outcome,
                "result_retained": false,
            }),
            Self::Refused { reason } => json!({
                "answer": answers::REFUSED,
                "reason": reason.as_str(),
            }),
            Self::Unclassified { body, .. } => body.clone(),
        }
    }

    /// Every decoded answer ends the caller's retries.
    pub fn is_final(&self) -> bool {
        true
    }

    /// Whether the answer guarantees the call's action has not run and never
    /// will: `withdrawn`, or a refusal (a denied or expired approval, an ended
    /// scope). `unknown_call` is not such a guarantee: it says only that no
    /// record exists.
    pub fn guarantees_not_run(&self) -> bool {
        matches!(self, Self::Withdrawn | Self::Refused { .. })
    }
}

fn member_str<'a>(
    object: &'a Map<String, Value>,
    answer: &'static str,
    member: &'static str,
) -> Result<&'a str, WithdrawDecodeError> {
    object
        .get(member)
        .and_then(Value::as_str)
        .ok_or(WithdrawDecodeError::BadMember { answer, member })
}

/// What a withdrawing caller concludes from one reply, and whether it
/// retries.
#[derive(Clone, Debug, PartialEq)]
pub enum WithdrawVerdict {
    /// `withdrawn` or `refused`: the call never ran and never will.
    NeverRan,
    /// `completed`: the call ran.
    Ran,
    /// `already_started` with an observed outcome code.
    StartedWithOutcome(String),
    /// `already_started` with outcome `unknown`: record `outcome_unknown`.
    OutcomeUnknown,
    /// `unknown_call`: the provider held nothing under that key.
    NothingHeld,
    /// A reply body this crate cannot classify (an unknown `answer`, or not an
    /// answer at all): record it verbatim; final.
    Unclassified(Value),
    /// A terminal route error: the four withdraw caller errors, or any other
    /// code that is not transient. Record it; do not retry.
    Terminal(ErrorBody),
    /// A transient route refusal ([`errors::is_transient`]): retry the same
    /// request.
    Retry(ErrorBody),
}

impl WithdrawVerdict {
    /// Only [`WithdrawVerdict::Retry`] sends the request again.
    pub fn retries(&self) -> bool {
        matches!(self, Self::Retry(_))
    }
}

/// The caller policy for one `tool.withdraw` reply: `Ok` is a `RESPONSE`
/// body, `Err` a route error.
pub fn caller_verdict(reply: Result<&Value, &ErrorBody>) -> WithdrawVerdict {
    match reply {
        Err(error) if errors::is_transient(&error.code) => WithdrawVerdict::Retry(error.clone()),
        Err(error) => WithdrawVerdict::Terminal(error.clone()),
        Ok(body) => match WithdrawAnswer::decode(body) {
            Ok(WithdrawAnswer::Withdrawn | WithdrawAnswer::Refused { .. }) => {
                WithdrawVerdict::NeverRan
            }
            Ok(WithdrawAnswer::Completed { .. }) => WithdrawVerdict::Ran,
            Ok(WithdrawAnswer::AlreadyStarted {
                outcome: StartedOutcome::Known(code),
            }) => WithdrawVerdict::StartedWithOutcome(code),
            Ok(WithdrawAnswer::AlreadyStarted {
                outcome: StartedOutcome::Unknown,
            }) => WithdrawVerdict::OutcomeUnknown,
            Ok(WithdrawAnswer::UnknownCall) => WithdrawVerdict::NothingHeld,
            Ok(WithdrawAnswer::Unclassified { body, .. }) => WithdrawVerdict::Unclassified(body),
            Err(_) => WithdrawVerdict::Unclassified(body.clone()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    fn scope() -> ScopeIdentity {
        ScopeIdentity {
            owner: "reserved:owner".into(),
            scope_ref: "r1".into(),
            scope_epoch: 3,
        }
    }

    #[test]
    fn known_withdraw_answers_decode_as_recorded() {
        let vectors = vectors::load("withdraw-answers.json");
        for case in vectors["known"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let answer = WithdrawAnswer::decode(&case["body"]).expect(name);
            let kind = match &answer {
                WithdrawAnswer::Withdrawn => "withdrawn",
                WithdrawAnswer::AlreadyStarted {
                    outcome: StartedOutcome::Unknown,
                } => "already_started_unknown",
                WithdrawAnswer::AlreadyStarted { .. } => "already_started",
                WithdrawAnswer::Completed {
                    result: CompletedResult::Retained(_),
                } => "completed_retained",
                WithdrawAnswer::Completed { .. } => "completed_outcome_only",
                WithdrawAnswer::Refused {
                    reason: RefusalReason::Other(_),
                } => "refused_other",
                WithdrawAnswer::Refused { .. } => "refused",
                WithdrawAnswer::UnknownCall => "unknown_call",
                WithdrawAnswer::Unclassified { .. } => "unclassified",
            };
            assert_eq!(kind, case["decodes_as"].as_str().unwrap(), "{name}");
            assert_eq!(answer.encode(), case["body"], "{name} re-encodes");
        }
        for case in vectors["malformed"].as_array().unwrap() {
            assert!(
                WithdrawAnswer::decode(&case["body"]).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn unknown_answers_decode_as_final_unclassified() {
        let vectors = vectors::load("withdraw-answers.json");
        let cases = vectors["unclassified"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let body = &case["body"];
            let answer = WithdrawAnswer::decode(body).expect("an unknown answer still decodes");
            assert_eq!(
                answer,
                WithdrawAnswer::Unclassified {
                    answer: body["answer"].as_str().unwrap().to_owned(),
                    body: body.clone(),
                }
            );
            assert!(answer.is_final());
            assert!(!answer.guarantees_not_run());
            assert_eq!(&answer.encode(), body, "recorded verbatim");
        }
    }

    #[test]
    fn a_withdraw_request_with_its_own_or_a_malformed_key_is_refused() {
        let mut request = WithdrawArguments::new("k1").into_request();
        assert_eq!(request.call_key, None);
        assert_eq!(
            parse_withdraw_request(&request),
            Ok(WithdrawArguments::new("k1"))
        );
        request.call_key = Some("k2".into());
        let error = parse_withdraw_request(&request).unwrap_err();
        assert_eq!(errors::invalid_request_field(&error), Some(CALL_KEY_FIELD));

        let request = WithdrawArguments::new("has space").into_request();
        let error = parse_withdraw_request(&request).unwrap_err();
        assert_eq!(
            errors::invalid_request_field(&error),
            Some(TARGET_CALL_KEY_FIELD)
        );
    }

    #[test]
    fn caller_rules_resolve_the_carrier_or_name_the_route_error() {
        let scope = scope();
        let scoped = Some(&scope);
        let bare = WithdrawArguments::new("k");
        let named = |c: &str| bare.clone().with_carrier(c);
        let carrier = |c: &str| Ok(CarrierResolution::Carrier(c.into()));

        assert_eq!(
            resolve_carrier("reserved:broca", scoped, &bare),
            carrier("reserved:broca")
        );
        assert_eq!(
            resolve_carrier("reserved:broca", scoped, &named("reserved:broca")),
            carrier("reserved:broca")
        );
        assert_eq!(
            resolve_carrier("reserved:broca", scoped, &named("reserved:other")),
            Err(CallerProblem::CarrierMismatch {
                named: "reserved:other".into()
            })
        );
        assert_eq!(
            resolve_carrier("reserved:owner", scoped, &bare),
            Ok(CarrierResolution::OwnerAsCarrier("reserved:owner".into()))
        );
        assert_eq!(
            resolve_carrier("reserved:owner", scoped, &named("reserved:broca")),
            carrier("reserved:broca")
        );
        let mut other_scope = bare.clone();
        other_scope.scope = Some(ScopeIdentity {
            scope_epoch: 4,
            ..scope.clone()
        });
        assert_eq!(
            resolve_carrier("reserved:broca", scoped, &other_scope),
            Err(CallerProblem::ScopeMismatch)
        );
        // Unscoped: only the carrier, keyed on the route principal; the
        // principal that owns some scope is no owner here.
        assert_eq!(
            resolve_carrier("reserved:owner", None, &bare),
            carrier("reserved:owner")
        );
        assert_eq!(
            resolve_carrier("direct", None, &named("reserved:broca")),
            Err(CallerProblem::CarrierMismatch {
                named: "reserved:broca".into()
            })
        );
        assert_eq!(
            resolve_carrier("direct", None, &other_scope),
            Err(CallerProblem::ScopeMismatch)
        );

        assert_eq!(check_record_scope(scoped, scoped), Ok(()));
        assert_eq!(check_record_scope(None, None), Ok(()));
        assert_eq!(
            check_record_scope(scoped, None),
            Err(CallerProblem::NotPermitted)
        );
        assert_eq!(
            check_record_scope(None, scoped),
            Err(CallerProblem::NotPermitted)
        );
    }

    #[test]
    fn every_caller_problem_has_its_own_route_error() {
        let codes = [
            CallerProblem::ScopeMismatch.error().code,
            CallerProblem::CarrierRequired.error().code,
            CallerProblem::CarrierMismatch { named: "x".into() }
                .error()
                .code,
            CallerProblem::NotPermitted.error().code,
        ];
        assert_eq!(
            codes,
            [
                errors::WITHDRAW_SCOPE_MISMATCH,
                errors::WITHDRAW_CARRIER_REQUIRED,
                errors::WITHDRAW_CARRIER_MISMATCH,
                errors::WITHDRAW_NOT_PERMITTED,
            ]
        );
    }

    #[test]
    fn caller_policy_vectors_decide_as_recorded() {
        let vectors = vectors::load("withdraw-answers.json");
        for case in vectors["policy"].as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let verdict = match (&case["body"], &case["route_error"]) {
                (body, Value::Null) => caller_verdict(Ok(body)),
                (_, error) => {
                    let error: ErrorBody = serde_json::from_value(error.clone()).unwrap();
                    caller_verdict(Err(&error))
                }
            };
            let kind = match &verdict {
                WithdrawVerdict::NeverRan => "never_ran",
                WithdrawVerdict::Ran => "ran",
                WithdrawVerdict::StartedWithOutcome(_) => "started_with_outcome",
                WithdrawVerdict::OutcomeUnknown => "outcome_unknown",
                WithdrawVerdict::NothingHeld => "nothing_held",
                WithdrawVerdict::Unclassified(_) => "unclassified",
                WithdrawVerdict::Terminal(_) => "terminal",
                WithdrawVerdict::Retry(_) => "retry",
            };
            assert_eq!(kind, case["verdict"].as_str().unwrap(), "{name}");
            assert_eq!(verdict.retries(), kind == "retry", "{name}");
        }
    }
}
