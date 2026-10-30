//! Stored Pi journal projection shared by runners and offline planners.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

/// A malformed stored journal. The stable error name is `pi_malformed_journal`;
/// `detail` identifies the offending entry or relationship. No path is guessed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PiJournalError {
    pub detail: String,
}

impl std::fmt::Display for PiJournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pi_malformed_journal: {}", self.detail)
    }
}
impl std::error::Error for PiJournalError {}

fn malformed(detail: impl Into<String>) -> PiJournalError {
    PiJournalError {
        detail: detail.into(),
    }
}

/// An ordinal-bearing entry, including entries that project no content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PiOrdinalEntry {
    pub entry_id: String,
    pub entry_type: String,
    pub ordinal: u64,
    pub part_count: usize,
    /// Model-visible content after context edits and `convertToLlm`, or null
    /// when nothing is projected. This is not a replacement for the served raw row.
    pub projected_content: Value,
}

/// The latest active-path native compaction. Its summary is never served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PiNativeCut {
    pub entry_id: String,
    pub first_kept_entry_id: String,
    /// The first ordinal at or after `first_kept_entry_id`. Earlier ordinals
    /// are outside the served range; the kept tail is never renumbered.
    pub first_kept_ordinal: u64,
}

/// The durable active path and all its ordinal-bearing entries, including the
/// prefix hidden by `native_cut`. Entry IDs in `active_path` include controls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PiSessionProjection {
    pub leaf_id: Option<String>,
    pub active_path: Vec<String>,
    pub entries: Vec<PiOrdinalEntry>,
    pub native_cut: Option<PiNativeCut>,
}

pub(super) fn content_count(content: &Value) -> Result<usize, String> {
    match content {
        Value::Null => Ok(0),
        Value::Array(parts) => Ok(parts.len()),
        Value::String(_) => Ok(1),
        Value::Object(reference)
            if reference.get("type").and_then(Value::as_str) == Some("text_ref") =>
        {
            Ok(1)
        }
        _ => Err("pi wrapped message content must be a string, array or text_ref".into()),
    }
}

fn valid_content(content: &Value) -> Result<Value, String> {
    if content.is_null() {
        return Err("pi wrapped message content must be a string, array or text_ref".into());
    }
    content_count(content)?;
    Ok(content.clone())
}

/// Pi's shell text, including empty output, cancellation, exit and truncation.
/// Mirrors messages.js:21-38 in the checksum-pinned Pi 1.3.0/1.4.0 packages.
pub(crate) fn pi_bash_text(message: &Value) -> Result<String, String> {
    let command = message["command"]
        .as_str()
        .ok_or("pi bashExecution command must be a string")?;
    let output = message["output"]
        .as_str()
        .ok_or("pi bashExecution output must be a string")?;
    let mut text = format!("Ran `{command}`\n");
    if output.is_empty() {
        text.push_str("(no output)");
    } else {
        text.push_str(&format!("```\n{output}\n```"));
    }
    if message["cancelled"] == true {
        text.push_str("\n\n(command cancelled)");
    } else if let Some(code) = message["exitCode"].as_i64().filter(|code| *code != 0) {
        text.push_str(&format!("\n\nCommand exited with code {code}"));
    }
    if message["truncated"] == true {
        if let Some(path) = message["fullOutputPath"]
            .as_str()
            .filter(|path| !path.is_empty())
        {
            text.push_str(&format!("\n\n[Output truncated. Full output: {path}]"));
        }
    }
    Ok(text)
}

/// Content produced by the entry's runtime message, then `convertToLlm`.
/// Raw standalone messages remain strict; the session helper normalizes old
/// missing/null message content as Pi's session loader does.
pub(crate) fn projected_content(entry: &Value) -> Result<Value, String> {
    match entry["type"].as_str() {
        Some("message") => {
            let message = &entry["message"];
            match message["role"].as_str() {
                Some("bashExecution") => {
                    let text = pi_bash_text(message)?;
                    if message["excludeFromContext"] == true {
                        return Ok(Value::Null);
                    }
                    let mut blocks = vec![json!({"type":"text", "text":text})];
                    if let Some(images) = message.get("images").and_then(Value::as_array) {
                        blocks.extend(images.iter().cloned());
                    }
                    Ok(Value::Array(blocks))
                }
                Some("system" | "user" | "assistant" | "toolResult") => {
                    valid_content(&message["content"])
                }
                _ => Err("pi wrapped message content must be a string, array or text_ref".into()),
            }
        }
        Some("custom_message") => valid_content(&entry["content"]),
        Some("branch_summary") => {
            let summary = entry["summary"]
                .as_str()
                .ok_or("pi branch_summary summary must be a string")?;
            if summary.is_empty() {
                return Ok(Value::Null);
            }
            Ok(
                json!([{"type":"text", "text":format!("The following is a summary of a branch that this conversation came back from:\n\n<summary>\n{summary}</summary>")}]),
            )
        }
        _ => Err("pi wrapped message content must be a string, array or text_ref".into()),
    }
}

fn ordinal_bearing(entry: &Value) -> bool {
    matches!(
        entry["type"].as_str(),
        Some("message" | "custom_message" | "branch_summary")
    )
}

/// Project a parsed stored Pi journal (an optional leading `session` header,
/// followed by entries in file order). Unlike Pi's permissive loader, malformed
/// IDs, parents, edits, content and cut targets return `pi_malformed_journal`.
///
/// The last non-header entry of any type, including `label`, is the leaf. Walk
/// its `parentId` chain to the root; abandoned branches and unpersisted in-memory
/// branch switches do not participate. A journal may hold several roots (an
/// append after Pi's `resetLeaf()` starts a new one); only the leaf's chain is
/// active. Every `message`, `custom_message` and
/// `branch_summary` on that path consumes one zero-based ordinal. All other
/// types, including settings, labels, context edits and compactions, consume none.
///
/// Apply the last active-path `context_edit` for each target before projecting
/// runtime messages through Pi's `convertToLlm` rules. A null replacement removes
/// content; replacements affect user, assistant, toolResult and custom content,
/// not shell/system/branch-summary content. A `bashExecution` with
/// `excludeFromContext: true` (Pi's `!!` shell-command prefix) and
/// removed entries retain their ordinals with zero parts. Count projected blocks:
/// strings and whole-content text refs count one, arrays count immediate blocks.
/// A plain shell execution renders one text block with command and output;
/// shell image attachments render separate image blocks, never extra text.
/// Ordinary images also retain their own parts. Details and other metadata are
/// not parts. `custom_message` uses top-level `content`; `branch_summary` uses
/// `summary`, rendered with Pi's branch-summary wrapper, and an empty `summary`
/// projects nothing (zero parts, ordinal kept).
///
/// The latest active-path compaction is the native cut. Neither it nor its
/// summary consumes an ordinal or is served. Entries before `firstKeptEntryId`
/// fall outside the served range; kept ordinals do not change. This helper
/// describes a snapshot: a late edit of already served model-visible content
/// requires a new lineage from that target's ordinal at a step boundary, never
/// in-place changes to served bytes. Serve raw wrapped journal entries, not
/// `projected_content`. Only `message` with runtime role `user` is the user's
/// own text; display keeps `custom`, `branchSummary` and `bashExecution`
/// runtime roles rather than relabeling them `user`.
///
/// Source: Pi 1.3.0 and 1.4.0 session-manager.js:124-280,732-753 and
/// messages.js:21-122, from the checksum-pinned packages listed with the vectors.
pub fn pi_session_projection(journal: &[Value]) -> Result<PiSessionProjection, PiJournalError> {
    let mut entries: Vec<&Value> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (position, entry) in journal.iter().enumerate() {
        let kind = entry["type"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| malformed(format!("entry {position} has no type")))?;
        let id = entry["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| malformed(format!("entry {position} has no id")))?;
        if kind == "session" {
            if position != 0 {
                return Err(malformed("session header must be first"));
            }
            continue;
        }
        if index.contains_key(id) {
            return Err(malformed(format!("duplicate id {id}")));
        }
        match entry.get("parentId") {
            Some(Value::Null) => {}
            Some(Value::String(parent)) if index.contains_key(parent.as_str()) => {}
            _ => {
                return Err(malformed(format!(
                    "{id} has a missing, forward or invalid parentId"
                )))
            }
        }
        if ordinal_bearing(entry) {
            normalized_content(entry).map_err(|detail| malformed(format!("{id}: {detail}")))?;
        }
        if kind == "compaction" {
            let kept = entry["firstKeptEntryId"]
                .as_str()
                .ok_or_else(|| malformed(format!("{id} has no firstKeptEntryId")))?;
            let mut ancestor = entry["parentId"].as_str();
            while let Some(parent) = ancestor {
                if parent == kept {
                    break;
                }
                ancestor = entries[index[parent]]["parentId"].as_str();
            }
            if ancestor.is_none() || !entry["summary"].is_string() {
                return Err(malformed(format!(
                    "{id} has an invalid compaction summary or kept-path target"
                )));
            }
        }
        if kind == "context_edit" {
            let target = entry["targetId"]
                .as_str()
                .and_then(|target| index.get(target).copied())
                .ok_or_else(|| malformed(format!("{id} has an unknown edit target")))?;
            if !ordinal_bearing(entries[target]) {
                return Err(malformed(format!(
                    "{id} edit target is not ordinal-bearing"
                )));
            }
            match entry.get("replacement") {
                Some(Value::Null) => {}
                Some(replacement) => {
                    valid_content(&replacement["content"])
                        .map_err(|detail| malformed(format!("{id}: {detail}")))?;
                }
                None => return Err(malformed(format!("{id} has no replacement"))),
            }
        }
        index.insert(id, entries.len());
        entries.push(entry);
    }
    let mut path = Vec::new();
    let mut current = entries.last().copied();
    while let Some(entry) = current {
        path.push(entry);
        current = entry["parentId"].as_str().map(|id| entries[index[id]]);
    }
    path.reverse();
    let edits: HashMap<_, _> = path
        .iter()
        .filter(|e| e["type"] == "context_edit")
        .map(|e| {
            (
                e["targetId"].as_str().expect("validated edit"),
                &e["replacement"],
            )
        })
        .collect();
    let mut projected = Vec::new();
    for entry in &path {
        if !ordinal_bearing(entry) {
            continue;
        }
        let id = entry["id"].as_str().expect("validated id");
        let mut edited = (*entry).clone();
        let content = match edits.get(id) {
            Some(Value::Null) => Value::Null,
            Some(replacement) => {
                if entry["type"] == "custom_message" {
                    edited["content"] = replacement["content"].clone();
                } else if matches!(
                    entry.pointer("/message/role").and_then(Value::as_str),
                    Some("user" | "assistant" | "toolResult")
                ) {
                    let content = &replacement["content"];
                    edited["message"]["content"] =
                        if entry["message"]["role"] != "user" && content.is_string() {
                            json!([{"type":"text", "text":content}])
                        } else {
                            content.clone()
                        };
                }
                normalized_content(&edited).map_err(malformed)?
            }
            None => normalized_content(entry).map_err(malformed)?,
        };
        projected.push(PiOrdinalEntry {
            entry_id: id.to_owned(),
            entry_type: entry["type"].as_str().expect("validated type").to_owned(),
            ordinal: projected.len() as u64,
            part_count: content_count(&content).map_err(malformed)?,
            projected_content: content,
        });
    }
    let native_cut = path
        .iter()
        .rev()
        .find(|entry| entry["type"] == "compaction")
        .map(|cut| {
            let id = cut["id"].as_str().expect("validated id");
            let kept = cut["firstKeptEntryId"]
                .as_str()
                .ok_or_else(|| malformed(format!("{id} has no firstKeptEntryId")))?;
            let kept_position = path
                .iter()
                .position(|entry| entry["id"] == kept)
                .filter(|position| {
                    *position < path.iter().position(|entry| entry["id"] == id).unwrap()
                })
                .ok_or_else(|| {
                    malformed(format!(
                        "{id} firstKeptEntryId is not a preceding active-path entry"
                    ))
                })?;
            let first_kept_ordinal = path[..kept_position]
                .iter()
                .filter(|e| ordinal_bearing(e))
                .count() as u64;
            Ok(PiNativeCut {
                entry_id: id.to_owned(),
                first_kept_entry_id: kept.to_owned(),
                first_kept_ordinal,
            })
        })
        .transpose()?;
    Ok(PiSessionProjection {
        leaf_id: entries
            .last()
            .map(|entry| entry["id"].as_str().unwrap().to_owned()),
        active_path: path
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect(),
        entries: projected,
        native_cut,
    })
}

fn normalized_content(entry: &Value) -> Result<Value, String> {
    let mut entry = entry.clone();
    if entry["type"] == "message" && entry["message"]["content"].is_null() {
        match entry["message"]["role"].as_str() {
            Some("system") => entry["message"]["content"] = json!(""),
            Some("user" | "assistant" | "toolResult") => entry["message"]["content"] = json!([]),
            _ => {}
        }
    }
    if entry["type"] == "custom_message" && entry["content"].is_null() {
        entry["content"] = json!([]);
    }
    projected_content(&entry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn pi_recording_and_synthetic_sessions_match_every_expected_result() {
        let file = vectors::load("pi-session.json");
        let journal: Vec<Value> = vectors::file(file["recording"].as_str().unwrap())
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let actual = pi_session_projection(&journal).unwrap();
        assert_eq!(serde_json::to_value(&actual).unwrap(), file["expected"]);
        for abandoned in file["abandoned_ids"].as_array().unwrap() {
            assert!(!actual
                .active_path
                .contains(&abandoned.as_str().unwrap().to_owned()));
            assert!(actual
                .entries
                .iter()
                .all(|entry| entry.entry_id != abandoned.as_str().unwrap()));
        }
        for case in file["synthetic"].as_array().unwrap() {
            if let Some(template_id) = case["template_id"].as_str() {
                assert_eq!(
                    case["journal"][0]["customType"],
                    format!("cortexkit:{template_id}")
                );
                assert_eq!(case["journal"][0]["display"], true);
            }
            let actual = pi_session_projection(case["journal"].as_array().unwrap()).unwrap();
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                case["expected"],
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn pi_recorded_abandoned_branch_still_counts_when_it_is_the_durable_path() {
        let journal: Vec<Value> = vectors::file("pi-1.3.0-1.4.0.jsonl")
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        let projection = pi_session_projection(&journal[..16]).unwrap();
        let expected = [1, 1, 1, 2, 3, 2, 1, 1, 1, 0, 1, 1];
        assert_eq!(
            projection
                .entries
                .iter()
                .map(|entry| entry.part_count)
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(projection.leaf_id.as_deref(), Some("entry-015"));
        assert!(projection.native_cut.is_none());
        for entry in &journal[1..16] {
            if ordinal_bearing(entry) {
                assert_eq!(
                    crate::serializer_profile::top_level_part_count("pi", entry).unwrap(),
                    projection
                        .entries
                        .iter()
                        .find(|row| row.entry_id == entry["id"])
                        .unwrap()
                        .part_count
                );
            }
        }
    }

    #[test]
    fn pi_latest_cut_preserves_the_tail_ordinals_and_control_leaf() {
        let journal = vec![
            json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"prefix"}}),
            json!({"type":"compaction","id":"b","parentId":"a","summary":"old cut","firstKeptEntryId":"a"}),
            json!({"type":"message","id":"c","parentId":"b","message":{"role":"user","content":"tail"}}),
            json!({"type":"compaction","id":"d","parentId":"c","summary":"new cut","firstKeptEntryId":"c"}),
            json!({"type":"label","id":"e","parentId":"d","targetId":"c","label":"leaf"}),
        ];
        let actual = pi_session_projection(&journal).unwrap();
        assert_eq!(actual.leaf_id.as_deref(), Some("e"));
        assert_eq!(
            actual
                .entries
                .iter()
                .map(|entry| entry.ordinal)
                .collect::<Vec<_>>(),
            [0, 1]
        );
        let cut = actual.native_cut.unwrap();
        assert_eq!(cut.entry_id, "d");
        assert_eq!(cut.first_kept_entry_id, "c");
        assert_eq!(cut.first_kept_ordinal, 1);
    }

    #[test]
    fn pi_follows_only_the_leaf_root_when_a_reset_started_a_second_root() {
        let journal = vec![
            json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"first root"}}),
            json!({"type":"message","id":"b","parentId":"a","message":{"role":"assistant","content":"reply"}}),
            json!({"type":"message","id":"c","parentId":null,"message":{"role":"user","content":"after reset"}}),
            json!({"type":"label","id":"d","parentId":"c","targetId":"c","label":"x"}),
        ];
        let projection = pi_session_projection(&journal).unwrap();
        let ids: Vec<&str> = projection
            .entries
            .iter()
            .map(|e| e.entry_id.as_str())
            .collect();
        assert_eq!(ids, ["c"]);
        assert_eq!(projection.entries[0].ordinal, 0);
    }

    #[test]
    fn pi_journals_refuse_malformed_relationships_and_content_by_name() {
        let root = json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":"a"}});
        for journal in [
            vec![json!({})],
            vec![root.clone(), root.clone()],
            vec![json!({"type":"label","id":"a","parentId":"missing"})],
            vec![json!({"type":"label","id":"a","parentId":"a"})],
            vec![root.clone(), json!({"type":"label","id":"b"})],
            vec![root.clone(), json!({"type":"session","id":"header"})],
            vec![
                root.clone(),
                json!({"type":"context_edit","id":"b","parentId":"a","targetId":"absent","replacement":null}),
            ],
            vec![
                root.clone(),
                json!({"type":"context_edit","id":"b","parentId":"a","targetId":"a","replacement":{"content":42}}),
            ],
            vec![
                root.clone(),
                json!({"type":"compaction","id":"b","parentId":"a","firstKeptEntryId":"absent"}),
            ],
            vec![
                json!({"type":"message","id":"a","parentId":null,"message":{"role":"user","content":42}}),
            ],
            vec![
                json!({"type":"message","id":"a","parentId":null,"message":{"role":"bashExecution","output":""}}),
            ],
            vec![json!({"type":"branch_summary","id":"a","parentId":null,"content":"invented"})],
        ] {
            let error = pi_session_projection(&journal).unwrap_err();
            assert!(
                error.to_string().starts_with("pi_malformed_journal: "),
                "{error}"
            );
        }
        assert_eq!(pi_session_projection(&[]).unwrap().entries, []);
    }

    #[test]
    fn pi_edits_preserve_runtime_projection_rules_and_ignore_abandoned_edits() {
        let journal = vec![
            json!({"type":"message","id":"a","parentId":null,"message":{"role":"assistant","content":null}}),
            json!({"type":"context_edit","id":"b","parentId":"a","targetId":"a","replacement":{"content":"abandoned"}}),
            json!({"type":"context_edit","id":"c","parentId":"a","targetId":"a","replacement":{"content":"last"}}),
            json!({"type":"custom_message","id":"d","parentId":"c","content":null,"display":false}),
            json!({"type":"context_edit","id":"e","parentId":"d","targetId":"d","replacement":{"content":"custom"}}),
            json!({"type":"message","id":"f","parentId":"e","message":{"role":"bashExecution","command":"pwd","output":"/fixture"}}),
            json!({"type":"context_edit","id":"g","parentId":"f","targetId":"f","replacement":{"content":"ignored"}}),
            json!({"type":"branch_summary","id":"h","parentId":"g","summary":""}),
        ];
        let actual = pi_session_projection(&journal).unwrap();
        assert_eq!(
            actual.entries[0].projected_content,
            json!([{"type":"text","text":"last"}])
        );
        assert_eq!(actual.entries[1].projected_content, json!("custom"));
        assert_eq!(
            actual.entries[2].projected_content,
            json!([{"type":"text","text":"Ran `pwd`\n```\n/fixture\n```"}])
        );
        assert_eq!(actual.entries[3].part_count, 0);
        assert_eq!(
            actual
                .entries
                .iter()
                .map(|entry| entry.ordinal)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3]
        );
    }
}
