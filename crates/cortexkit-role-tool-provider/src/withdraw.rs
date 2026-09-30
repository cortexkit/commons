//! `tool.withdraw`: withdrawing a call the provider holds.
//!
//! A provider that can hold a call past its own reply (an approval-gated call
//! waiting for its answer, say) serves `tool.withdraw`. The request is an
//! ordinary tool request on the provider's tool route, named `tool.withdraw`;
//! it is not a model tool. The call to withdraw is `arguments.call_key`, and
//! the provider's record for it is keyed `(carrier, call_key)`, where the
//! carrier is the principal that raised the call.
//!
//! Who may withdraw: the carrier itself, or the owner of the call's scope.
//! - The carrier's request needs no `arguments.carrier`: the carrier is the
//!   route's stamped principal. Naming a different carrier is refused.
//! - The owner's request must name `arguments.carrier`, because the owner is
//!   not the carrier and the record cannot be found without it.
//! - `arguments.scope`, when present, must equal the route's stamped scope.
//! - A caller that is neither the carrier nor the owner of the record's scope
//!   gets the route error `withdraw_not_permitted`, never an answer.
//!
//! The answer is read from the provider's record, so it is identical for
//! every permitted caller and every repeat. `unknown_call` means the provider
//! holds no record under that `(carrier, call_key)`; it is never a refusal and
//! never means "not sent".

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use subc_protocol::ErrorBody;

use crate::{
    call::{KeyedToolCallRequest, CALL_KEY_FIELD},
    errors, ops,
    scope::ScopeIdentity,
};

/// Answer names.
pub mod answers {
    pub const WITHDRAWN: &str = "withdrawn";
    pub const ALREADY_STARTED: &str = "already_started";
    pub const COMPLETED: &str = "completed";
    pub const REFUSED: &str = "refused";
    pub const UNKNOWN_CALL: &str = "unknown_call";
}

/// Refusal reasons the role names. The reason is an open string.
pub mod reasons {
    pub const DENIED: &str = "denied";
    pub const EXPIRED: &str = "expired";
    pub const SCOPE_ENDED: &str = "scope_ended";
}

/// The outcome an `already_started` answer carries when the provider cannot
/// tell whether the call settled. Never replaced by a guessed success.
pub const OUTCOME_UNKNOWN: &str = "unknown";

/// The arguments of a `tool.withdraw` request.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
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
    pub fn new(call_key: impl Into<String>) -> Self {
        Self {
            call_key: call_key.into(),
            carrier: None,
            scope: None,
        }
    }

    pub fn with_carrier(mut self, carrier: impl Into<String>) -> Self {
        self.carrier = Some(carrier.into());
        self
    }

    /// The request that carries these arguments. It has no top-level
    /// `call_key`: that would name the withdraw request itself, which is not
    /// a call anyone can hold.
    pub fn into_request(self) -> KeyedToolCallRequest {
        KeyedToolCallRequest::new(
            ops::TOOL_WITHDRAW,
            serde_json::to_value(self).expect("withdraw arguments serialize"),
        )
    }
}

/// The provider's first check of a `tool.withdraw` request: a top-level
/// `call_key` is refused as `invalid_request {field: "call_key"}`, and
/// arguments that do not decode as `invalid_request {field: "arguments"}`.
pub fn parse_withdraw_request(
    request: &KeyedToolCallRequest,
) -> Result<WithdrawArguments, ErrorBody> {
    if request.call_key.is_some() {
        return Err(errors::invalid_request(
            CALL_KEY_FIELD,
            "tool.withdraw names its target in arguments.call_key and carries no call_key of its own",
        ));
    }
    serde_json::from_value(request.request.arguments.clone())
        .map_err(|error| errors::invalid_request("arguments", error.to_string()))
}

/// Why a withdraw caller's request is refused before any record is read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallerProblem {
    /// `arguments.scope` differs from the route's stamped scope.
    ScopeMismatch,
    /// The scope's owner did not name `arguments.carrier`.
    OwnerMustNameCarrier,
    /// A caller that is not the scope's owner named a carrier other than
    /// itself. That is a failed caller check: `withdraw_not_permitted`.
    CarrierMismatch { named: String },
}

/// Decide whose record a withdraw request addresses, from the route's
/// stamped principal and scope and the request's arguments.
///
/// On success the provider reads the record `(carrier, arguments.call_key)`.
/// If that record exists under a scope other than `stamp_scope`, the caller
/// check fails too, and the provider answers `withdraw_not_permitted`.
pub fn resolve_carrier(
    caller: &str,
    stamp_scope: &ScopeIdentity,
    arguments: &WithdrawArguments,
) -> Result<String, CallerProblem> {
    if arguments
        .scope
        .as_ref()
        .is_some_and(|named| named != stamp_scope)
    {
        return Err(CallerProblem::ScopeMismatch);
    }
    if caller == stamp_scope.owner {
        return arguments
            .carrier
            .clone()
            .ok_or(CallerProblem::OwnerMustNameCarrier);
    }
    match &arguments.carrier {
        Some(named) if named != caller => Err(CallerProblem::CarrierMismatch {
            named: named.clone(),
        }),
        _ => Ok(caller.to_owned()),
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
/// `answer` this crate does not know, which the caller records verbatim.
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
    fn a_withdraw_request_with_its_own_call_key_is_refused() {
        let request = WithdrawArguments::new("k1")
            .into_request()
            .with_call_key("k2");
        let error = parse_withdraw_request(&request).unwrap_err();
        assert_eq!(errors::invalid_request_field(&error), Some(CALL_KEY_FIELD));

        let request = WithdrawArguments::new("k1").into_request();
        assert_eq!(request.call_key, None);
        assert_eq!(
            parse_withdraw_request(&request),
            Ok(WithdrawArguments::new("k1"))
        );
    }

    #[test]
    fn the_carrier_comes_from_the_stamp_and_the_owner_must_name_it() {
        let scope = scope();
        let bare = WithdrawArguments::new("k");
        assert_eq!(
            resolve_carrier("reserved:broca", &scope, &bare),
            Ok("reserved:broca".into())
        );
        assert_eq!(
            resolve_carrier(
                "reserved:broca",
                &scope,
                &bare.clone().with_carrier("reserved:broca")
            ),
            Ok("reserved:broca".into())
        );
        assert_eq!(
            resolve_carrier(
                "reserved:broca",
                &scope,
                &bare.clone().with_carrier("reserved:other")
            ),
            Err(CallerProblem::CarrierMismatch {
                named: "reserved:other".into()
            })
        );
        assert_eq!(
            resolve_carrier("reserved:owner", &scope, &bare),
            Err(CallerProblem::OwnerMustNameCarrier)
        );
        assert_eq!(
            resolve_carrier(
                "reserved:owner",
                &scope,
                &bare.clone().with_carrier("reserved:broca")
            ),
            Ok("reserved:broca".into())
        );
        let mut other_scope = bare;
        other_scope.scope = Some(ScopeIdentity {
            scope_epoch: 4,
            ..scope.clone()
        });
        assert_eq!(
            resolve_carrier("reserved:broca", &scope, &other_scope),
            Err(CallerProblem::ScopeMismatch)
        );
    }
}
