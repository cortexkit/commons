//! Dense session ordinals over OpenCode 2's stored SQL message rows.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

/// Malformed session rows. The stable error name is `opencode_v2_malformed_session`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OpenCodeV2SessionError {
    pub detail: String,
}

impl std::fmt::Display for OpenCodeV2SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "opencode_v2_malformed_session: {}", self.detail)
    }
}

impl std::error::Error for OpenCodeV2SessionError {}

fn malformed(detail: impl Into<String>) -> OpenCodeV2SessionError {
    OpenCodeV2SessionError {
        detail: detail.into(),
    }
}

/// One stored non-compaction row, including rows that project no native units.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OpenCodeV2OrdinalEntry {
    pub entry_id: String,
    pub entry_type: String,
    pub ordinal: u64,
    pub part_count: usize,
    /// Native units, not provider messages or a replacement for the served body.
    pub projected_content: Value,
}

/// The latest completed native compaction and its inclusive boundary.
/// A compaction with a self-referencing `keepID` is a runner marker instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OpenCodeV2NativeCut {
    pub entry_id: String,
    /// `metadata.keepID`, or the compaction's own id when it is absent.
    pub boundary_entry_id: String,
    /// The first ordinal at or after the boundary; may equal the entry count.
    pub first_kept_ordinal: u64,
}

/// The latest completed self-referencing compaction. Its row id identifies
/// the runner's own checkpoint; SQL rows do not reveal that checkpoint's cut.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OpenCodeV2RunnerMarker {
    pub entry_id: String,
}

/// All ordinal-bearing rows, including the prefix hidden by a native cut.
/// A runner marker requires the runner's checkpoint to determine served range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct OpenCodeV2SessionProjection {
    pub entries: Vec<OpenCodeV2OrdinalEntry>,
    pub native_cut: Option<OpenCodeV2NativeCut>,
    pub runner_marker: Option<OpenCodeV2RunnerMarker>,
}

fn text(value: &Value) -> Result<Value, String> {
    if value.is_string() || value.get("type").and_then(Value::as_str) == Some("text_ref") {
        Ok(json!({"type":"text", "text":value}))
    } else {
        Err("opencode-v2 text must be a string or text_ref".into())
    }
}

fn array<'a>(body: &'a Value, name: &str) -> Result<&'a [Value], String> {
    match body.get(name) {
        None => Ok(&[]),
        Some(Value::Array(values)) => Ok(values),
        _ => Err(format!("opencode-v2 {name} must be an array")),
    }
}

pub(crate) fn shell_text(body: &Value) -> Result<String, String> {
    let command = body["command"]
        .as_str()
        .ok_or("opencode-v2 shell has no command")?;
    let output = body["output"]["output"].as_str().unwrap_or("");
    Ok(format!("The following shell command was executed by the user:\n\nCommand:\n{command}\n\nOutput:\n{output}"))
}

/// Native content locations in their projection order. Replay state stays opaque.
/// File expansion, unsupported-media dropping and image deduplication do not
/// change native part identities.
pub(crate) fn projected_content(body: &Value) -> Result<Value, String> {
    let mut units = Vec::new();
    match body["type"].as_str() {
        Some("user") => {
            for skill in array(body, "skills")? {
                if let Some(supplied) = skill.get("text") {
                    units.push(text(supplied)?);
                }
            }
            let supplied = body.get("text").ok_or("opencode-v2 user has no text")?;
            if supplied.as_str() != Some("") {
                units.push(text(supplied)?);
            }
            units.extend_from_slice(array(body, "files")?);
        }
        Some("synthetic" | "system" | "skill") => units.push(text(&body["text"])?),
        Some("location-switched") => {
            let directory = body["location"]["directory"]
                .as_str()
                .ok_or("opencode-v2 location-switched has no directory")?;
            units.push(json!({"type":"text", "text":format!("The working directory has been changed to {directory}.")}));
        }
        Some("shell") => {
            if body["metadata"]["background"] != true {
                units.push(json!({"type":"text", "text":shell_text(body)?}));
            }
        }
        Some("assistant") => {
            let content = body["content"].as_array().ok_or(
                "opencode-v2 assistant content must be an array (hydrate stored parts first)",
            )?;
            for slot in content {
                if !matches!(slot["type"].as_str(), Some("text" | "reasoning" | "tool")) {
                    return Err("opencode-v2 assistant slot must be text, reasoning or tool".into());
                }
            }
            units.extend_from_slice(content);
        }
        // Model notices depend on the current model upstream. Counting none
        // keeps an already served row stable when the session changes model.
        Some("model-switched" | "agent-switched" | "idle" | "compaction") => {}
        _ => return Err("opencode-v2 body has no supported native type".into()),
    }
    Ok(Value::Array(units))
}

/// Assign ordinals in native `seq` order, never id or timestamp order. The
/// caller supplies one session's SQL envelopes `{id,type,seq,data,...}`, after
/// hydrating any separate part rows and blobs. Compactions consume no ordinal.
/// Entries describe storage, not read eligibility: unfinished messages must
/// still be withheld under the contract's finished-only read rule (§4.2).
pub fn opencode_v2_session_projection(
    rows: &[Value],
) -> Result<OpenCodeV2SessionProjection, OpenCodeV2SessionError> {
    let mut ordered = Vec::with_capacity(rows.len());
    let mut sequences = HashSet::new();
    let mut ids = HashMap::new();
    let mut session = None;
    for row in rows {
        let id = row["id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .ok_or_else(|| malformed("missing id"))?;
        let seq = row["seq"]
            .as_u64()
            .ok_or_else(|| malformed(format!("{id}: missing or invalid seq")))?;
        if !sequences.insert(seq) {
            return Err(malformed(format!("duplicate seq {seq}")));
        }
        if ids.insert(id, seq).is_some() {
            return Err(malformed(format!("duplicate id {id}")));
        }
        if let Some(current) = row.get("session_id") {
            if !current.is_string() || session.is_some_and(|previous| previous != current) {
                return Err(malformed("rows do not belong to one session"));
            }
            session = Some(current);
        }
        let kind = row["type"]
            .as_str()
            .ok_or_else(|| malformed(format!("{id}: missing type")))?;
        let data = row["data"]
            .as_object()
            .ok_or_else(|| malformed(format!("{id}: missing data object")))?;
        if data.get("id").is_some_and(|v| v.as_str() != Some(id))
            || data.get("type").is_some_and(|v| v.as_str() != Some(kind))
        {
            return Err(malformed(format!("{id}: conflicting data identity")));
        }
        ordered.push((seq, id, kind, data));
    }
    ordered.sort_by_key(|row| row.0);
    let mut entries = Vec::new();
    let mut latest = None;
    let mut runner_marker = None;
    for (seq, id, kind, data) in &ordered {
        if *kind == "compaction" {
            let status = data.get("status").and_then(Value::as_str);
            if !matches!(status, Some("running" | "failed" | "completed")) {
                return Err(malformed(format!("{id}: invalid compaction status")));
            }
            let keep_id = match data
                .get("metadata")
                .and_then(|metadata| metadata.get("keepID"))
            {
                None => None,
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| malformed(format!("{id}: invalid keepID")))?,
                ),
            };
            let boundary = keep_id.unwrap_or(id);
            let boundary_seq = *ids
                .get(boundary)
                .ok_or_else(|| malformed(format!("{id}: keepID names no row: {boundary}")))?;
            if status == Some("completed") {
                if keep_id == Some(*id) {
                    runner_marker = Some(OpenCodeV2RunnerMarker {
                        entry_id: (*id).to_owned(),
                    });
                } else {
                    latest = Some((*id, boundary, boundary_seq));
                }
            }
            continue;
        }
        let mut body = Value::Object((*data).clone());
        body["id"] = json!(id);
        body["type"] = json!(kind);
        let content =
            projected_content(&body).map_err(|error| malformed(format!("{id}: {error}")))?;
        entries.push((
            *seq,
            OpenCodeV2OrdinalEntry {
                entry_id: (*id).to_owned(),
                entry_type: (*kind).to_owned(),
                ordinal: entries.len() as u64,
                part_count: content.as_array().expect("projected native units").len(),
                projected_content: content,
            },
        ));
    }
    let native_cut = latest.map(|(id, boundary, seq)| OpenCodeV2NativeCut {
        entry_id: id.to_owned(),
        boundary_entry_id: boundary.to_owned(),
        first_kept_ordinal: entries
            .iter()
            .position(|(entry_seq, _)| *entry_seq >= seq)
            .unwrap_or(entries.len()) as u64,
    });
    Ok(OpenCodeV2SessionProjection {
        entries: entries.into_iter().map(|(_, entry)| entry).collect(),
        native_cut,
        runner_marker,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        serializer_profile::{top_level_part_count, OPENCODE_V2},
        vectors,
    };

    fn decoded(row: &Value) -> Value {
        let mut body = row["data"].clone();
        body["id"] = row["id"].clone();
        body["type"] = row["type"].clone();
        body
    }

    fn check(case: &Value, rows: &[Value]) {
        let actual = opencode_v2_session_projection(rows).unwrap();
        assert_eq!(
            serde_json::to_value(&actual).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
        let outside: Vec<_> = actual
            .entries
            .iter()
            .filter(|entry| {
                actual
                    .native_cut
                    .as_ref()
                    .is_some_and(|cut| entry.ordinal < cut.first_kept_ordinal)
            })
            .map(|entry| entry.entry_id.clone())
            .collect();
        if case["outside_served_range"].is_null() {
            // A runner marker cannot reveal the runner's checkpoint boundary.
            assert!(actual.runner_marker.is_some());
            assert!(actual.native_cut.is_none());
        } else {
            assert_eq!(
                json!(outside),
                case["outside_served_range"],
                "{}",
                case["name"]
            );
        }
        for entry in &actual.entries {
            let stored = rows.iter().find(|row| row["id"] == entry.entry_id).unwrap();
            assert_eq!(
                top_level_part_count(OPENCODE_V2, &decoded(stored)).unwrap(),
                entry.part_count,
                "{}",
                entry.entry_id
            );
        }
        for control in rows.iter().filter(|row| row["type"] == "compaction") {
            assert!(!actual
                .entries
                .iter()
                .any(|entry| control["id"] == entry.entry_id));
            assert_eq!(
                top_level_part_count(OPENCODE_V2, &decoded(control)).unwrap(),
                0
            );
        }
        // Input order is irrelevant; native sequence numbers alone determine it.
        let mut reversed = rows.to_vec();
        reversed.reverse();
        assert_eq!(opencode_v2_session_projection(&reversed).unwrap(), actual);
    }

    #[test]
    fn recorded_sessions_reproduce_every_expected_field() {
        let cases = vectors::load("opencode-v2-session.json");
        for case in cases["recorded"].as_array().unwrap() {
            let capture = vectors::load(case["recording"].as_str().unwrap());
            let rows = capture["snapshots"][case["snapshot"].as_str().unwrap()]
                .as_array()
                .unwrap();
            check(case, rows);
        }
    }

    #[test]
    fn synthetic_controls_and_unfinished_slots_reproduce_every_expected_field() {
        let cases = vectors::load("opencode-v2-session.json");
        for case in cases["synthetic"].as_array().unwrap() {
            assert_eq!(case["synthetic"], true);
            check(case, case["rows"].as_array().unwrap());
        }
    }

    #[test]
    fn recorded_stale_fork_keep_ids_are_refused_by_name() {
        let cases = vectors::load("opencode-v2-session.json");
        for case in cases["recorded_refused"].as_array().unwrap() {
            let capture = vectors::load(case["recording"].as_str().unwrap());
            let rows = capture["snapshots"][case["snapshot"].as_str().unwrap()]
                .as_array()
                .unwrap();
            let error = opencode_v2_session_projection(rows).unwrap_err();
            assert_eq!(
                error.to_string(),
                case["error"].as_str().unwrap(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn recorded_runner_markers_never_infer_cuts_and_reverts_fall_back() {
        for version in ["2.0.26", "2.2.3"] {
            let capture = vectors::load(&format!("opencode-v2-reinjection-rows-{version}.json"));
            let project = |stage: &str| {
                opencode_v2_session_projection(capture["snapshots"][stage].as_array().unwrap())
                    .unwrap()
            };
            let current = project("current_marker");
            assert_eq!(
                current.runner_marker.unwrap().entry_id,
                "msg_s0_current_marker"
            );
            assert!(current.native_cut.is_none());
            assert_eq!(current.entries.len(), 23);
            assert_eq!(current.entries[0].ordinal, 0);
            let whole = project("below.committed");
            assert!(whole.runner_marker.is_none());
            assert!(whole.native_cut.is_none());
            assert_eq!(whole.entries.len(), 3);
            if version == "2.2.3" {
                let before = project("between.before");
                let after = project("between.committed");
                assert_ne!(before.runner_marker, after.runner_marker);
                assert!(after.runner_marker.is_some());
                assert!(after.native_cut.is_none());
                assert_eq!(after.entries, before.entries[..14]);
                assert_eq!(whole.entries, project("below.before").entries[..3]);
            }
        }
    }

    #[test]
    fn model_switched_count_stays_zero_as_the_session_changes_model() {
        let mut rows = vec![json!({"id":"original", "type":"model-switched", "seq":1,
            "data":{"model":{"providerID":"fixture", "id":"first"}}})];
        for (index, model) in ["first", "other", "first"].iter().enumerate() {
            rows.push(
                json!({"id":format!("switch-{index}"), "type":"model-switched", "seq":index+2,
                "data":{"model":{"providerID":"fixture", "id":model}}}),
            );
            let actual = opencode_v2_session_projection(&rows).unwrap();
            assert_eq!(actual.entries[0].entry_id, "original");
            assert!(actual.entries.iter().all(|entry| entry.part_count == 0));
            assert_eq!(
                top_level_part_count(OPENCODE_V2, &decoded(&rows[0])).unwrap(),
                0
            );
        }
    }

    #[test]
    fn malformed_sessions_are_named_and_never_guessed() {
        let user = json!({"id":"user", "type":"user", "seq":1, "data":{"text":"Fixture"}});
        let mut missing_seq = user.clone();
        missing_seq.as_object_mut().unwrap().remove("seq");
        let mut duplicate_seq = user.clone();
        duplicate_seq["id"] = json!("other");
        let mut duplicate_id = user.clone();
        duplicate_id["seq"] = json!(2);
        let mut conflicting = user.clone();
        conflicting["data"]["type"] = json!("assistant");
        let mut bad_content = user.clone();
        bad_content["data"]["text"] = json!(false);
        let cut = json!({"id":"cut", "type":"compaction", "seq":2, "data":{"status":"completed", "metadata":{"keepID":"absent"}}});
        for (rows, detail) in [
            (vec![missing_seq], "seq"),
            (vec![user.clone(), duplicate_seq], "duplicate seq"),
            (vec![user.clone(), duplicate_id], "duplicate id"),
            (vec![conflicting], "conflicting data identity"),
            (vec![bad_content], "text"),
            (vec![user, cut], "keepID names no row"),
            (
                vec![json!({"role":"tool", "tool_call_id":"projection-only"})],
                "missing id",
            ),
            (
                vec![json!({"id":"partial", "seq":1, "type":"assistant", "data":{}})],
                "hydrate stored parts first",
            ),
        ] {
            let error = opencode_v2_session_projection(&rows)
                .unwrap_err()
                .to_string();
            assert!(
                error.starts_with("opencode_v2_malformed_session:"),
                "{error}"
            );
            assert!(error.contains(detail), "{error}");
        }
        assert_eq!(opencode_v2_session_projection(&[]).unwrap().entries, []);
        assert_eq!(
            top_level_part_count(OPENCODE_V2, &json!({"type":"user", "text":""})).unwrap(),
            0
        );
    }

    #[test]
    fn hydration_and_replay_state_do_not_add_parts() {
        // Synthetic already-hydrated SQL data: the caller restored separate
        // parts in their native order before passing the row to this helper.
        for replay in [
            json!({"providerState":{"signature":"opaque"}}),
            json!({"native":{"signature":"opaque"}}),
        ] {
            let mut slot = json!({"type":"tool", "id":"call", "name":"fixture", "state":{"status":"completed", "input":{}, "content":[{"type":"text", "text":"Outcome"}]}});
            slot.as_object_mut()
                .unwrap()
                .extend(replay.as_object().unwrap().clone());
            let rows = vec![
                json!({"id":"hydrated", "seq":4, "type":"assistant", "data":{"content":[{"type":"text", "text":"First"},slot]}}),
            ];
            let actual = opencode_v2_session_projection(&rows).unwrap();
            assert_eq!(actual.entries[0].part_count, 2);
            assert_eq!(
                actual.entries[0].projected_content,
                rows[0]["data"]["content"]
            );
        }
    }
}
