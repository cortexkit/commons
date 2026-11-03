//! OpenCode 1's entry projection, including completed-compaction-pair handling,
//! Every active-path host message counts, including an empty dead step; only
//! completed compaction pairs are markers. `cortexkit-role-compaction` W4.1/W4.2
//! are the authoritative entry-selection rules.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// A counted OpenCode 1 row and its model-visible parts. Marker pairs have
/// no row here. Empty entries still consume an ordinal.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct OpenCode1OrdinalEntry {
    pub ordinal: u64,
    pub mid: String,
    pub projected_content: Vec<Value>,
}

impl OpenCode1OrdinalEntry {
    pub fn new(ordinal: u64, mid: impl Into<String>, projected_content: Vec<Value>) -> Self {
        Self {
            ordinal,
            mid: mid.into(),
            projected_content,
        }
    }
}

/// Compaction W4.2, OpenCode 1: "every `message` row of the session, by
/// `(time_created, id)`. A dead step (no parts, not completed, no error) is an
/// entry with no parts." Only a completed compaction pair is a marker: its
/// opener holds `compaction`, its assistant summary has `summary: true`, a
/// `finish` (any value), and no error. An unfinished pair's opener renders
/// "What did we do so far?" and its summary is an ordinary entry.
///
/// Supply hydrated `{info, parts}` rows; `info.time.created` is the host's
/// `time_created`, and the summary's `parentID` identifies its opener.
pub fn opencode1_session_projection(
    messages: &[Value],
) -> Result<Vec<OpenCode1OrdinalEntry>, String> {
    let mut ordered = Vec::with_capacity(messages.len());
    let mut ids = HashSet::new();
    for message in messages {
        let info = &message["info"];
        let id = info["id"]
            .as_str()
            .ok_or("opencode-aisdk message has no id")?;
        let time = info["time"]["created"]
            .as_u64()
            .ok_or("opencode-aisdk message has no creation time")?;
        if !ids.insert(id) {
            return Err("duplicate opencode-aisdk message id".into());
        }
        if !message["parts"].is_array() {
            return Err("opencode-aisdk message has no parts array".into());
        }
        ordered.push((time, id, message));
    }
    ordered.sort_by_key(|(time, id, _)| (*time, *id));
    let mut markers = HashSet::new();
    for (_, id, message) in &ordered {
        let info = &message["info"];
        if info["role"] != "assistant"
            || info["summary"] != true
            || info.get("finish").is_none()
            || !info["error"].is_null()
        {
            continue;
        }
        if let Some(parent) = info["parentID"].as_str() {
            if ordered.iter().any(|(_, opener_id, opener)| {
                *opener_id == parent
                    && opener["info"]["role"] == "user"
                    && opener["parts"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|part| part["type"] == "compaction")
            }) {
                markers.insert(parent);
                markers.insert(*id);
            }
        }
    }
    let mut entries = Vec::new();
    for (_, id, message) in ordered {
        if markers.contains(id) {
            continue;
        }
        let info = &message["info"];
        let parts = message["parts"].as_array().unwrap();
        let abort = info["error"]["name"] == "MessageAbortedError";
        let error_has_content = abort
            && parts
                .iter()
                .any(|p| !matches!(p["type"].as_str(), Some("step-start" | "reasoning")));
        let mut projected = Vec::new();
        if info["role"] != "assistant" || info["error"].is_null() || error_has_content {
            for part in parts {
                match part["type"].as_str() {
                    Some("text") if (info["role"] == "user" && part["ignored"] == true) || part["text"].as_str() == Some("") => {},
                    Some("file") if matches!(part["mime"].as_str(), Some("text/plain" | "application/x-directory")) => {},
                    Some("compaction") => projected.push(json!({"type":"text", "text":"What did we do so far?"})),
                    Some("subtask") => projected.push(json!({"type":"text", "text":"The following tool was executed by the user"})),
                    Some("text" | "reasoning" | "tool" | "file") => projected.push(part.clone()),
                    _ => {},
                }
            }
        }
        entries.push(OpenCode1OrdinalEntry::new(
            entries.len() as u64 + 1,
            id,
            projected,
        ));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, role: &str, parts: Value) -> Value {
        json!({"info":{"id":id,"role":role,"time":{"created":1}},"parts":parts})
    }

    #[test]
    fn opencode1_w4_2_counts_dead_and_unfinished_pairs_but_not_completed_markers() {
        let dead = row("a", "assistant", json!([]));
        let opener = row("b", "user", json!([{"type":"compaction"}]));
        let mut summary = row("c", "assistant", json!([{"type":"text","text":"summary"}]));
        summary["info"]["parentID"] = json!("b");
        summary["info"]["summary"] = json!(true);
        let entries =
            opencode1_session_projection(&[summary.clone(), opener.clone(), dead.clone()]).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|e| (e.ordinal, e.mid.as_str()))
                .collect::<Vec<_>>(),
            [(1, "a"), (2, "b"), (3, "c")]
        );
        assert!(entries[0].projected_content.is_empty());
        assert_eq!(
            entries[1].projected_content[0]["text"],
            "What did we do so far?"
        );
        summary["info"]["finish"] = json!("length");
        let entries = opencode1_session_projection(&[dead, opener, summary]).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].ordinal, 1);
    }

    #[test]
    fn opencode1_w4_2_projects_errors_ignored_files_and_subtasks() {
        let mut failed = row(
            "a",
            "assistant",
            json!([{"type":"text","text":"not rendered"}]),
        );
        failed["info"]["error"] = json!({"name":"ProviderError"});
        let mut aborted = row("b", "assistant", json!([{"type":"text","text":"kept"}]));
        aborted["info"]["error"] = json!({"name":"MessageAbortedError"});
        let user = row(
            "c",
            "user",
            json!([
                {"type":"text","text":"ignored","ignored":true}, {"type":"text","text":""},
                {"type":"file","mime":"text/plain"}, {"type":"file","mime":"application/x-directory"},
                {"type":"subtask"}, {"type":"file","mime":"image/png"}
            ]),
        );
        let entries = opencode1_session_projection(&[user, aborted, failed]).unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|entry| entry.ordinal)
                .collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(entries[0].projected_content.is_empty());
        assert_eq!(entries[1].projected_content[0]["text"], "kept");
        assert_eq!(entries[2].projected_content.len(), 2);
        let assistant = row(
            "d",
            "assistant",
            json!([{"type":"text","text":"visible","ignored":true}]),
        );
        assert_eq!(
            opencode1_session_projection(&[assistant]).unwrap()[0].projected_content[0]["text"],
            "visible"
        );
        assert_eq!(
            entries[2].projected_content[0]["text"],
            "The following tool was executed by the user"
        );
    }
}
