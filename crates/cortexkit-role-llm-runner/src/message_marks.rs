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
    /// Registry name when sent, for display only; never an identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl SpeaksFor {
    /// Construct a fact. Send admission still validates the id.
    pub fn new(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            name: None,
        }
    }

    /// Set the display name. Admission validates it and requires
    /// `notice_with_speaks_for` even when no notice mark is sent.
    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
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
    /// Owner-marked notice carrying a person's words; false is absent.
    /// Set only after [`crate::send::SendRequest::check_notice_keep`] succeeds.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub keep: bool,
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
    check_with_notice_speaks_for(params, advertised, false, scope_owner, sender)
}

/// Validate marks with the additive `notice_with_speaks_for` group. That group
/// requires `message_marks` and enables both names and combined notice marks.
/// The legacy [`check`] keeps refusing either extension.
pub fn check_with_notice_speaks_for(
    params: &Map<String, Value>,
    advertised: bool,
    notice_with_speaks_for: bool,
    scope_owner: Option<&str>,
    sender: &str,
) -> Result<MessageMarks, &'static str> {
    if advertised
        && !notice_with_speaks_for
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
        if object.keys().any(|key| key != "agent_id" && key != "name") {
            return Err("speaks_for");
        }
        let mut speaker = SpeaksFor::new(id);
        if let Some(raw_name) = object.get("name") {
            if !notice_with_speaks_for {
                return Err("speaks_for.name");
            }
            let name = raw_name
                .as_str()
                .filter(|name| {
                    (1..=64).contains(&name.len())
                        && name.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
                })
                .ok_or("speaks_for.name")?;
            speaker.name = Some(name.into());
        }
        marks.speaks_for = Some(speaker);
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

/// Compare marks on two admitted sends sharing a `send_id`. A differing field
/// names a `send_id_reuse` refusal before any write. Compare `speaks_for` as the
/// whole object; false `notice` and `keep` equal absence. Marks remain outside
/// checksums: this comparison is retry identity only.
pub fn retry_difference(
    accepted: &Map<String, Value>,
    resent: &Map<String, Value>,
) -> Option<&'static str> {
    if accepted.get("speaks_for") != resent.get("speaks_for") {
        return Some("speaks_for");
    }
    ["notice", "keep"].into_iter().find(|field| {
        (accepted.get(*field) == Some(&Value::Bool(true)))
            != (resent.get(*field) == Some(&Value::Bool(true)))
    })
}

#[cfg(all(test, feature = "test-vectors"))]
mod tests {
    use super::*;

    #[test]
    fn speaks_for_name_round_trips() {
        let wire = serde_json::json!({"agent_id":"agent-a", "name":"Ada"});
        let mark: SpeaksFor = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(mark).unwrap(), wire);
    }

    #[test]
    fn named_message_marks_vectors_validate_declarations() {
        let file: Value =
            serde_json::from_str(crate::vectors::file("message-marks-named.json").unwrap())
                .unwrap();
        for case in file["cases"].as_array().unwrap() {
            let result = check_with_notice_speaks_for(
                case["params"].as_object().unwrap(),
                true,
                case["declared"].as_bool().unwrap(),
                Some("owner"),
                "owner",
            );
            match case["field"].as_str() {
                Some(field) => assert_eq!(result.unwrap_err(), field, "{}", case["name"]),
                None => assert_eq!(
                    serde_json::to_value(result.unwrap()).unwrap(),
                    case["params"],
                    "{}",
                    case["name"]
                ),
            }
        }
    }

    #[test]
    fn marks_retry_identity_uses_whole_speaker_and_normalized_booleans() {
        let file: Value =
            serde_json::from_str(crate::vectors::file("message-marks-retry.json").unwrap())
                .unwrap();
        for case in file["cases"].as_array().unwrap() {
            assert_eq!(
                retry_difference(
                    case["accepted"].as_object().unwrap(),
                    case["resent"].as_object().unwrap(),
                ),
                case["field"].as_str(),
                "{}",
                case["name"]
            );
        }
    }
}
