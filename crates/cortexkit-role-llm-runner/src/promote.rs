//! Atomically move the caller's queued send into the running turn.
//!
//! Session identity comes from the bound route, as for `session.send_status`.
//! Promotion preserves the send's identity and uses §9's ordinary steer and
//! queued-send receipt rules; its outcome is not a new send state.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Strict `session.promote` parameters. Build with [`Self::new`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PromoteRequest {
    /// An earlier send in the caller's own send-id namespace.
    pub send_id: String,
    /// Durable replay key for this promotion, not a replacement send id.
    pub request_id: String,
}

impl PromoteRequest {
    pub fn new(send_id: impl Into<String>, request_id: impl Into<String>) -> Self {
        Self {
            send_id: send_id.into(),
            request_id: request_id.into(),
        }
    }
}

/// Validate before writing; errors name the `invalid_params` detail field.
pub fn check_request(params: &Map<String, Value>) -> Result<PromoteRequest, String> {
    for key in params.keys() {
        if !matches!(key.as_str(), "send_id" | "request_id") {
            return Err(key.clone());
        }
    }
    if params.get("send_id").and_then(Value::as_str).is_none() {
        return Err("send_id".into());
    }
    let id = params
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or("request_id")?;
    if id.is_empty() || id.len() > 128 || !id.bytes().all(|b| (0x20..=0x7e).contains(&b)) {
        return Err("request_id".into());
    }
    serde_json::from_value(Value::Object(params.clone())).map_err(|_| "send_id".into())
}

/// Why a queued send was left queued. Unknown reasons never confirm promotion.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum PromoteReason {
    NoActiveRun,
    Unknown(String),
}

impl From<String> for PromoteReason {
    fn from(value: String) -> Self {
        match value.as_str() {
            "no_active_run" => Self::NoActiveRun,
            _ => Self::Unknown(value),
        }
    }
}

impl From<PromoteReason> for String {
    fn from(value: PromoteReason) -> Self {
        match value {
            PromoteReason::NoActiveRun => "no_active_run".into(),
            PromoteReason::Unknown(value) => value,
        }
    }
}

/// The durable promotion decision, separate from [`crate::send::SendReply`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(try_from = "WireReply", into = "WireReply")]
#[non_exhaustive]
pub enum PromoteReply {
    /// Still queued with a run active: now follows §9's steer rule.
    Promoted,
    /// Already started or joined a turn; no additional delivery occurs.
    Delivered,
    /// Still queued and unchanged, with no active run.
    NotPromoted { reason: PromoteReason },
    /// Missing in this caller's namespace, or ended without delivery.
    NotQueued,
    /// A future outcome is retained but never confirms promotion or delivery.
    Unknown(String),
}

#[derive(Deserialize, Serialize)]
struct WireReply {
    outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reason: Option<PromoteReason>,
}

impl TryFrom<WireReply> for PromoteReply {
    type Error = &'static str;

    fn try_from(value: WireReply) -> Result<Self, Self::Error> {
        Ok(match value.outcome.as_str() {
            "promoted" => Self::Promoted,
            "delivered" => Self::Delivered,
            "not_promoted" => Self::NotPromoted {
                reason: value.reason.ok_or("not_promoted requires reason")?,
            },
            "not_queued" => Self::NotQueued,
            _ => Self::Unknown(value.outcome),
        })
    }
}

impl From<PromoteReply> for WireReply {
    fn from(value: PromoteReply) -> Self {
        let (outcome, reason) = match value {
            PromoteReply::Promoted => ("promoted".into(), None),
            PromoteReply::Delivered => ("delivered".into(), None),
            PromoteReply::NotPromoted { reason } => ("not_promoted".into(), Some(reason)),
            PromoteReply::NotQueued => ("not_queued".into(), None),
            PromoteReply::Unknown(value) => (value, None),
        };
        Self { outcome, reason }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;
    use serde_json::json;

    #[test]
    fn send_promote_vectors_round_trip() {
        let file = vectors::load("send-promote.json");
        let mut seen = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "cases") {
            let name = case["name"].as_str().unwrap();
            let request: PromoteRequest = vectors::round_trip(name, &case["request"]);
            assert_eq!(
                check_request(case["request"].as_object().unwrap()),
                Ok(request)
            );
            vectors::round_trip::<PromoteReply>(name, &case["reply"]);
            seen.insert(case["reply"]["outcome"].as_str().unwrap());
        }
        assert_eq!(
            seen,
            ["promoted", "delivered", "not_promoted", "not_queued"].into()
        );
        assert_eq!(
            serde_json::to_value(PromoteRequest::new("send-1", "promote-1")).unwrap(),
            json!({"send_id":"send-1", "request_id":"promote-1"})
        );
        for case in vectors::cases(&file, "refused_requests") {
            assert_eq!(
                check_request(case["request"].as_object().unwrap()).unwrap_err(),
                case["field"].as_str().unwrap()
            );
        }
        assert!(serde_json::from_value::<PromoteRequest>(
            json!({"send_id":"s", "request_id":"r", "session":"other"})
        )
        .is_err());
    }

    #[test]
    fn send_promote_replies_keep_future_outcomes_and_reasons() {
        for value in [
            json!({"outcome":"future"}),
            json!({"outcome":"not_promoted", "reason":"future"}),
        ] {
            let reply: PromoteReply = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(reply).unwrap(), value);
        }
        assert!(serde_json::from_value::<PromoteReply>(json!({"outcome":"not_promoted"})).is_err());
        assert_eq!(
            serde_json::from_value::<PromoteReply>(json!({"outcome":"promoted", "future":true}))
                .unwrap(),
            PromoteReply::Promoted
        );
    }

    #[test]
    fn send_promote_discovery_requires_op_and_steer() {
        use crate::{
            capabilities,
            describe::{check_describe, DescribeProblem},
            errors, ops, REQUIRED_OPS,
        };
        assert_eq!(
            capabilities::group_ops(capabilities::SEND_PROMOTE),
            Some([ops::SESSION_PROMOTE].as_slice())
        );
        let mut describe = json!({"majors":[{"version":"llm-runner/v1", "ops":([REQUIRED_OPS, &[ops::SESSION_PROMOTE, ops::SESSION_SEND]].concat())}],
            "implementation_version":"test", "session_capabilities_from":"admission", "capabilities":["send_promote", "steer"]});
        assert!(check_describe(&describe)
            .unwrap()
            .declares(capabilities::SEND_PROMOTE));
        describe["capabilities"] = json!(["send_promote"]);
        assert!(format!("{:?}", check_describe(&describe).unwrap_err())
            .contains("send_promote_without_steer"));
        describe["capabilities"] = json!(["send_promote", "steer"]);
        describe["majors"][0]["ops"] = json!([REQUIRED_OPS, &[ops::SESSION_SEND]].concat());
        assert!(check_describe(&describe).unwrap_err().contains(
            &DescribeProblem::GroupIncomplete {
                capability: capabilities::SEND_PROMOTE,
                op: ops::SESSION_PROMOTE,
            }
        ));
        let detail = json!({"field":"send_id"});
        assert_eq!(
            errors::refused_field(errors::REQUEST_ID_REUSE, Some(&detail)),
            Some("send_id")
        );
        for code in [errors::REQUEST_ID_REUSE, errors::UNSUPPORTED] {
            assert!(errors::CODES.contains(&code));
            assert!(!errors::is_retryable(code));
        }
    }
}
