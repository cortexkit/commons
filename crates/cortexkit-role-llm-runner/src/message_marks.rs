//! Positive display marks, stored beside bodies rather than inside them.
//!
//! Admission uses the daemon-attested sender and scope owner, never identities
//! supplied in the send. Honor every refusal before writing session state.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// An agent's words carried by the sending module.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SpeaksFor {
    pub agent_id: String,
}

impl SpeaksFor {
    /// Construct a fact. Send admission still validates the id.
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
        }
    }
}

/// Normalized marks to persist with an admitted entry or riding record.
/// False notices are absent on the wire and need not be stored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct MessageMarks {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaks_for: Option<SpeaksFor>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub notice: bool,
}

/// The fallback agent-id grammar: 1–128 printable ASCII bytes, including space.
pub fn agent_id_valid(id: &str) -> bool {
    (1..=128).contains(&id.len()) && id.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

/// Validate send members and carrier authority, returning an `invalid_params`
/// field on refusal. `scope_owner` is absent for an unscoped session; both
/// identities must come from the validated daemon route. This performs no writes.
pub fn check(
    params: &Map<String, Value>,
    advertised: bool,
    scope_owner: Option<&str>,
    sender: &str,
) -> Result<MessageMarks, &'static str> {
    if advertised
        && params.contains_key("speaks_for")
        && params.get("notice") == Some(&Value::Bool(true))
    {
        return Err("notice");
    }
    let mut marks = MessageMarks::default();
    if let Some(raw) = params.get("speaks_for") {
        if !advertised {
            return Err("speaks_for");
        }
        let object = raw.as_object().ok_or("speaks_for.agent_id")?;
        let id = object
            .get("agent_id")
            .and_then(Value::as_str)
            .filter(|id| agent_id_valid(id))
            .ok_or("speaks_for.agent_id")?;
        if object.len() != 1 {
            return Err("speaks_for");
        }
        marks.speaks_for = Some(SpeaksFor::new(id));
    }
    if let Some(raw) = params.get("notice") {
        if !advertised {
            return Err("notice");
        }
        marks.notice = raw.as_bool().ok_or("notice")?;
    }
    if marks.speaks_for.is_some() && scope_owner != Some(sender) {
        return Err("speaks_for");
    }
    Ok(marks)
}
