//! Shared person-steer and urgent-owner admission for handed-back calls.

use crate::{capabilities, delivery_order::PlanSender};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Refuse group-only sender marks before decoding, fetching or storing a plan.
/// Even false requires the mark's group. Also call [`check_plan_for_owner`]
/// with the session's scope owner to validate who may supply `yields`.
pub fn check_plan<S: AsRef<str>>(plan: &Map<String, Value>, groups: &[S]) -> Result<(), String> {
    let declared = groups
        .iter()
        .any(|group| group.as_ref() == capabilities::CALL_YIELD);
    let notice_yield = groups
        .iter()
        .any(|group| group.as_ref() == capabilities::NOTICE_YIELD);
    if let Some(senders) = plan.get("senders").and_then(Value::as_array) {
        for (index, sender) in senders.iter().enumerate() {
            if let Some(person) = sender.get("person") {
                if !declared || !person.is_boolean() {
                    return Err(format!("plan.senders[{index}].person"));
                }
            }
            if let Some(yields) = sender.get("yields") {
                if !notice_yield || !yields.is_boolean() {
                    return Err(format!("plan.senders[{index}].yields"));
                }
            }
        }
    }
    Ok(())
}

/// Only the session's scope owner (the plan composer) may supply `yields`.
/// An unscoped session has no such sender. Identity comes from route admission,
/// not a field supplied by the caller in the plan.
pub fn check_plan_for_owner<S: AsRef<str>>(
    plan: &Map<String, Value>,
    groups: &[S],
    scope_owner: Option<&str>,
) -> Result<(), String> {
    check_plan(plan, groups)?;
    if let Some(senders) = plan.get("senders").and_then(Value::as_array) {
        for (index, sender) in senders.iter().enumerate() {
            if sender.get("yields").is_some()
                && (scope_owner.is_none()
                    || sender.get("module").and_then(Value::as_str) != scope_owner)
            {
                return Err(format!("plan.senders[{index}].yields"));
            }
        }
    }
    Ok(())
}

/// The delivery forms shared by both hand-back triggers.
pub fn is_steer(delivery: &Value) -> bool {
    delivery.as_str() == Some("steer")
        || delivery
            .as_array()
            .and_then(|modes| modes.first())
            .and_then(Value::as_str)
            == Some("steer")
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
    is_steer(delivery)
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
