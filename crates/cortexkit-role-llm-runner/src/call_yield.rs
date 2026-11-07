//! Opt-in person-steer admission and durable identity for handed-back calls.

use crate::{capabilities, delivery_order::PlanSender};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Refuse any supplied `person` member before decoding or storing a plan when
/// `call_yield` is undeclared. Even false is a group-only member on the wire.
pub fn check_plan<S: AsRef<str>>(plan: &Map<String, Value>, groups: &[S]) -> Result<(), String> {
    let declared = groups
        .iter()
        .any(|group| group.as_ref() == capabilities::CALL_YIELD);
    if let Some(senders) = plan.get("senders").and_then(Value::as_array) {
        for (index, sender) in senders.iter().enumerate() {
            if let Some(person) = sender.get("person") {
                if !declared || !person.is_boolean() {
                    return Err(format!("plan.senders[{index}].person"));
                }
            }
        }
    }
    Ok(())
}

/// Whether an admitted send is the person's steer. Use the attested sender,
/// never a sender string supplied in the message body. Host-interface messages
/// delivered directly into an active run follow the same trigger rule.
pub fn is_person_steer(
    senders: &[PlanSender],
    attested_sender: &str,
    delivery: &Value,
    notice: bool,
    speaks_for: bool,
) -> bool {
    let steer = delivery.as_str() == Some("steer")
        || delivery
            .as_array()
            .and_then(|modes| modes.first())
            .and_then(Value::as_str)
            == Some("steer");
    steer
        && !notice
        && !speaks_for
        && senders
            .iter()
            .any(|sender| sender.module == attested_sender && sender.person)
}

/// Store with a yielded call so terminal late results can become notices even
/// after Stop or restart. A sent yield with no answer adds no such record.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct YieldedCall {
    pub carrier: String,
    pub call_key: String,
}

impl YieldedCall {
    pub fn new(carrier: impl Into<String>, call_key: impl Into<String>) -> Self {
        Self {
            carrier: carrier.into(),
            call_key: call_key.into(),
        }
    }
}
