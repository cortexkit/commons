//! Message-lineage anchors for runners declaring `message_checkpoint`.
//! Legacy tag-only anchoring remains in [`super::anchor`].

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{MarkerCheckpoint, SetupCheckpoint};
use crate::read::HeadCut;

/// The member of a Pi `compaction` entry's `details` that holds the
/// runner's [`MarkerCheckpoint`]. `details` may hold other members: an
/// extension's own record (standalone Magic Context writes `source` and
/// `lastCompactedOrdinal`), a runner's private records, or the
/// `details.cortexkit` record of an early draft, which the recorded
/// `entry-021` in `display-pi.json` carries. A reader takes only this member,
/// so none of those can be read as an anchor.
pub const PI_MARKER_CHECKPOINT_MEMBER: &str = "cortexkit_checkpoint";

/// The runner's checkpoint on one Pi journal entry, when it names `session`.
///
/// `None` when the entry isn't a `compaction`, has no
/// `details.cortexkit_checkpoint`, holds one that doesn't decode as a
/// [`MarkerCheckpoint`], or holds one naming another session (for example,
/// after a fork). Every other member of `details`, `details.cortexkit` included, is
/// ignored. It reads only this entry:
/// whether the entry is the in-effect compaction on the active path is the
/// caller's to decide, and a checkpoint missing `message_count` or
/// `head_cut` still cannot restore a message lineage.
pub fn pi_marker_checkpoint(entry: &Value, session: &str) -> Option<MarkerCheckpoint> {
    if entry.get("type").and_then(Value::as_str) != Some("compaction") {
        return None;
    }
    let member = entry.get("details")?.get(PI_MARKER_CHECKPOINT_MEMBER)?;
    let checkpoint: MarkerCheckpoint = serde_json::from_value(member.clone()).ok()?;
    (checkpoint.session == session).then_some(checkpoint)
}

/// The next message ordinal after a native cut. Overflow refuses the cut.
pub fn next_message_count(through_ordinal: u64) -> Option<u64> {
    through_ordinal.checked_add(1)
}

/// A host-id boundary, without reading or retaining the folded message.
///
/// Non-exhaustive: match it with a `_` arm.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "position", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckpointPosition {
    /// Numbering starts at the retained message `mid` (the checkpoint's `cut`).
    Before { mid: String },
    /// `cut` is gone: numbering starts right after the folded message `mid`.
    After { mid: String },
    /// No adoption cut survives: the legacy session-start fallback, tags only.
    SessionStart,
}

/// Where a surviving adoption lineage starts, and the cut it reports.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct MessageAnchor {
    /// The lineage's `first_ordinal`: the checkpoint's `message_count`.
    pub first_ordinal: u64,
    /// What `session.head.cut` reports for this lineage.
    pub head_cut: HeadCut,
    /// The host boundary numbering starts from.
    pub position: CheckpointPosition,
}

/// Separate message and tag anchors: tag fallback cannot reset a lineage.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct CheckpointAnchors {
    /// The message lineage, or `None` when no adoption cut survives.
    pub message: Option<MessageAnchor>,
    /// The highest tag already allocated; the next part takes this plus one.
    pub tag_checkpoint: u64,
    /// The host boundary tag numbering starts from.
    pub tag_position: CheckpointPosition,
}

/// Why a Setup checkpoint can't anchor a message lineage.
///
/// Non-exhaustive: match it with a `_` arm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MessageCheckpointError {
    /// `message_count` or `head_cut` is missing; a counter is never read as 0.
    MissingCounters,
    /// `cut` survives without the folded message before it: inconsistent state.
    HeadMismatch,
}

/// Resolve a complete Setup checkpoint for a `message_checkpoint` runner.
/// A missing folded-through message removes the adoption cut; Setup must
/// then be refused by the provider before a model call. A retained `cut`
/// without its predecessor is inconsistent, not a session-start fallback.
pub fn message_checkpoint_anchors(
    checkpoint: &SetupCheckpoint,
    cut_survives: bool,
    head_survives: bool,
) -> Result<CheckpointAnchors, MessageCheckpointError> {
    let (Some(first_ordinal), Some(head_cut)) =
        (checkpoint.message_count, checkpoint.head_cut.as_ref())
    else {
        return Err(MessageCheckpointError::MissingCounters);
    };
    if cut_survives && !head_survives {
        return Err(MessageCheckpointError::HeadMismatch);
    }
    let position = if cut_survives {
        CheckpointPosition::Before {
            mid: checkpoint.cut.clone(),
        }
    } else if head_survives {
        CheckpointPosition::After {
            mid: head_cut.mid.clone(),
        }
    } else {
        CheckpointPosition::SessionStart
    };
    Ok(CheckpointAnchors {
        message: head_survives.then(|| MessageAnchor {
            first_ordinal,
            head_cut: head_cut.clone(),
            position: position.clone(),
        }),
        tag_checkpoint: checkpoint.tag_checkpoint,
        tag_position: position,
    })
}

/// Applies only OpenCode 1's finished-summary rule to a message row's `data`.
/// This function cannot see the separately stored parts and does not inspect
/// a `parts` field in the JSON. Malformed JSON counts. Runners declaring
/// `message_checkpoint` must use [`opencode1_message_counts_with_parts`] for
/// the full §4.8 rule.
pub fn opencode1_message_counts(data: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
        return true;
    };
    // A summary is JSON `true` or any number equal to 1, so `1` and `1.0` both
    // match, as SQLite's numeric `json_extract(data, '$.summary') = 1` does.
    let summary = value["summary"] == true || value["summary"].as_f64() == Some(1.0);
    !(summary && value["finish"] == "stop")
}

/// Applies OpenCode 1's full §4.8 counting rule to a message and its stored parts.
/// Pass the message row's `data` and each stored part's `type`, in part order,
/// from the separate part rows keyed by that message's id.
/// Finished summaries, nonempty compaction-only messages and dead steps are
/// excluded from both counts and served reads. A dead step is an assistant
/// with no parts, no `time.completed` and no `error`: a step that ended before
/// the model call, which OpenCode 1 never sends to the model. Apply it to
/// settled rows only; while a step may still write to its assistant, that
/// assistant is unfinished and a runner never numbers past it. Other zero-part
/// messages, such as a finished assistant with an `error`, and malformed
/// message JSON count unless one of those exclusions applies. Consume message
/// rows in ascending `(time_created, id)` order.
pub fn opencode1_message_counts_with_parts<'a>(
    message_data: &str,
    part_types: impl IntoIterator<Item = &'a str>,
) -> bool {
    let mut part_types = part_types.into_iter();
    let Some(first) = part_types.next() else {
        return opencode1_message_counts(message_data) && !opencode1_dead_step(message_data);
    };
    let compaction_only =
        first == "compaction" && part_types.all(|part_type| part_type == "compaction");
    opencode1_message_counts(message_data) && !compaction_only
}

/// A part-less assistant row with no `time.completed` and no `error`. Absent
/// and `null` count as missing. Malformed JSON is never a dead step.
fn opencode1_dead_step(message_data: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(message_data) else {
        return false;
    };
    value["role"] == "assistant" && value["time"]["completed"].is_null() && value["error"].is_null()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_message_count_is_checked() {
        assert_eq!(next_message_count(0), Some(1));
        assert_eq!(next_message_count(u64::MAX - 1), Some(u64::MAX));
        assert_eq!(next_message_count(u64::MAX), None);
    }

    #[test]
    fn missing_counters_are_not_zero() {
        let head = HeadCut::new("p", "digest", 123);
        let mut checkpoint = SetupCheckpoint::new(500, "u");
        for (count, cut) in [(None, None), (Some(42), None), (None, Some(head))] {
            checkpoint.message_count = count;
            checkpoint.head_cut = cut;
            assert_eq!(
                message_checkpoint_anchors(&checkpoint, true, true),
                Err(MessageCheckpointError::MissingCounters)
            );
        }
    }

    #[test]
    fn setup_checkpoint_and_older_decoder_round_trips() {
        #[derive(Deserialize, Serialize)]
        struct OlderCheckpoint {
            tag_checkpoint: u64,
            cut: String,
        }
        let vectors = crate::vectors::load("message-checkpoint.json");
        for value in vectors["setup"].as_array().unwrap() {
            let decoded: SetupCheckpoint = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(decoded).unwrap(), *value);
            let older: OlderCheckpoint = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(older).unwrap(), vectors["setup"][0]);
        }
    }

    #[test]
    fn worked_head_cut_uses_ordinary_content_digest() {
        let vectors = crate::vectors::load("message-checkpoint.json");
        let worked = &vectors["worked_head_cut"];
        assert_eq!(
            serde_jcs::to_string(&worked["message"]).unwrap(),
            worked["jcs_utf8"]
        );
        assert_eq!(
            crate::content_digest::content_digest(&worked["message"]),
            worked["head_cut"]["digest"]
        );
        let cut: HeadCut = serde_json::from_value(worked["head_cut"].clone()).unwrap();
        assert_eq!(cut.written_at, 1_793_145_600_123);
    }

    #[test]
    fn opencode1_counting_vectors() {
        let vectors = crate::vectors::load("message-checkpoint.json");
        let mut rows = vectors["opencode1"]["rows"].as_array().unwrap().clone();
        rows.sort_by_key(|row| {
            (
                row["time_created"].as_u64().unwrap(),
                row["id"].as_str().unwrap().to_owned(),
            )
        });
        let parts: Vec<_> = vectors["opencode1"]["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| {
                (
                    part["message_id"].clone(),
                    serde_json::from_str::<serde_json::Value>(part["data"].as_str().unwrap())
                        .unwrap(),
                )
            })
            .collect();
        let ids: Vec<_> = rows
            .iter()
            .filter(|row| {
                opencode1_message_counts_with_parts(
                    row["data"].as_str().unwrap(),
                    parts
                        .iter()
                        .filter(|(message_id, _)| *message_id == row["id"])
                        .map(|(_, part)| part["type"].as_str().unwrap()),
                )
            })
            .map(|row| row["id"].clone())
            .collect();
        assert_eq!(
            ids,
            *vectors["opencode1"]["counted_ids"].as_array().unwrap()
        );
        assert_eq!(
            ids.len() as u64,
            vectors["opencode1"]["message_count"].as_u64().unwrap()
        );
        for (id, counts) in [
            ("marker-only", false),
            ("marker-and-text", true),
            ("empty-parts", true),
            ("dead-step", false),
            ("error-step", true),
            ("two-markers-only", false),
            ("summary-float", false),
        ] {
            let row = rows.iter().find(|row| row["id"] == id).unwrap();
            assert_eq!(
                opencode1_message_counts_with_parts(
                    row["data"].as_str().unwrap(),
                    parts
                        .iter()
                        .filter(|(message_id, _)| *message_id == row["id"])
                        .map(|(_, part)| part["type"].as_str().unwrap()),
                ),
                counts,
                "{id}"
            );
            assert_eq!(ids.iter().any(|served_id| served_id == id), counts, "{id}");
        }
        let summary_only_ids: Vec<_> = rows
            .iter()
            .filter(|row| opencode1_message_counts(row["data"].as_str().unwrap()))
            .map(|row| row["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            summary_only_ids,
            [
                "a",
                "d",
                "e",
                "f",
                "marker-only",
                "marker-and-text",
                "dead-step",
                "empty-parts",
                "two-markers-only",
                "error-step",
            ]
        );
    }

    #[test]
    fn opencode1_summary_only_compatibility_ignores_embedded_parts() {
        assert!(opencode1_message_counts(
            r#"{"role":"user","parts":[{"type":"compaction"}]}"#
        ));
        assert!(!opencode1_message_counts(
            r#"{"summary":true,"finish":"stop","parts":[{"type":"text"}]}"#
        ));
    }

    #[test]
    fn opencode1_separate_parts_apply_even_to_malformed_message_data() {
        assert!(opencode1_message_counts_with_parts("{malformed", []));
        assert!(!opencode1_message_counts_with_parts(
            r#"{"role":"assistant","time":{"created":1}}"#,
            []
        ));
        assert!(opencode1_message_counts_with_parts(
            r#"{"role":"assistant","time":{"created":1},"error":{"name":"APIError"}}"#,
            []
        ));
        assert!(opencode1_message_counts_with_parts(
            r#"{"role":"assistant","time":{"created":1,"completed":2}}"#,
            []
        ));
        assert!(opencode1_message_counts_with_parts(
            r#"{"role":"assistant","time":{"created":1}}"#,
            ["text"]
        ));
        assert!(opencode1_message_counts_with_parts(
            r#"{"role":"user"}"#,
            []
        ));
        assert!(!opencode1_message_counts_with_parts(
            "{malformed",
            ["compaction"]
        ));
        assert!(opencode1_message_counts_with_parts(
            "{malformed",
            ["compaction", "text"]
        ));
        assert!(!opencode1_message_counts_with_parts(
            r#"{"summary":1,"finish":"stop"}"#,
            ["text"]
        ));
    }

    #[test]
    fn opencode1_summary_accepts_any_number_equal_to_one() {
        for data in [
            r#"{"summary":true,"finish":"stop"}"#,
            r#"{"summary":1,"finish":"stop"}"#,
            r#"{"summary":1.0,"finish":"stop"}"#,
            r#"{"summary":1e0,"finish":"stop"}"#,
        ] {
            assert!(!opencode1_message_counts(data), "{data}");
        }
        for data in [
            r#"{"summary":"1","finish":"stop"}"#,
            r#"{"summary":1.5,"finish":"stop"}"#,
            r#"{"summary":0,"finish":"stop"}"#,
            r#"{"summary":1.0,"finish":"length"}"#,
        ] {
            assert!(opencode1_message_counts(data), "{data}");
        }
    }

    #[test]
    fn every_revert_table_row_has_separate_message_and_tag_anchors() {
        let vectors = crate::vectors::load("message-checkpoint.json");
        let checkpoint: SetupCheckpoint =
            serde_json::from_value(vectors["setup"][1].clone()).unwrap();
        for row in vectors["anchors"].as_array().unwrap() {
            let anchors = message_checkpoint_anchors(
                &checkpoint,
                row["cut_survives"].as_bool().unwrap(),
                row["head_survives"].as_bool().unwrap(),
            );
            if row.get("error").is_some() {
                assert_eq!(
                    anchors,
                    Err(MessageCheckpointError::HeadMismatch),
                    "{}",
                    row["name"]
                );
                continue;
            }
            let anchors = anchors.unwrap();
            assert_eq!(anchors.tag_checkpoint, 500, "{}", row["name"]);
            assert_eq!(
                serde_json::to_value(&anchors.tag_position).unwrap(),
                row["position"],
                "{}",
                row["name"]
            );
            assert_eq!(
                anchors.message.as_ref().map(|m| m.first_ordinal),
                row["first_ordinal"].as_u64(),
                "{}",
                row["name"]
            );
            if let Some(message) = anchors.message {
                assert_eq!(message.head_cut, checkpoint.head_cut.clone().unwrap());
                assert_eq!(message.position, anchors.tag_position);
            }
        }
    }

    #[test]
    fn pi_marker_checkpoint_reads_only_its_own_member() {
        let vectors = crate::vectors::load("message-checkpoint.json");
        let pi = &vectors["pi_marker"];
        let session = pi["session"].as_str().unwrap();
        let expected: MarkerCheckpoint = serde_json::from_value(pi["checkpoint"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&expected).unwrap(), pi["checkpoint"]);
        assert_eq!(pi_marker_checkpoint(&pi["after"], session), Some(expected));
        assert_eq!(pi_marker_checkpoint(&pi["before"], session), None);
        let cases = pi["no_anchor"].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            assert_eq!(
                pi_marker_checkpoint(&case["entry"], session),
                None,
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn pi_move_apply_vector_changes_only_the_cut_and_the_member() {
        let vectors = crate::vectors::load("message-checkpoint.json");
        let pi = &vectors["pi_marker"];
        let without_apply = |entry: &Value| {
            let mut entry = entry.clone();
            entry.as_object_mut().unwrap().remove("firstKeptEntryId");
            entry["details"]
                .as_object_mut()
                .unwrap()
                .remove(PI_MARKER_CHECKPOINT_MEMBER);
            entry
        };
        assert_ne!(
            pi["before"]["firstKeptEntryId"],
            pi["after"]["firstKeptEntryId"]
        );
        assert_eq!(without_apply(&pi["before"]), without_apply(&pi["after"]));
        assert_eq!(
            pi["after"]["details"][PI_MARKER_CHECKPOINT_MEMBER],
            pi["checkpoint"]
        );
    }

    #[test]
    fn recorded_pi_draft_records_are_no_anchor() {
        // The recorded Pi journal's compaction entry carries an early draft
        // record under `details.cortexkit`, which must never read as the
        // runner's checkpoint, whatever session it names.
        fn compactions<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
            match value {
                Value::Object(map) => {
                    if map.get("type").and_then(Value::as_str) == Some("compaction")
                        && map
                            .get("details")
                            .and_then(|d| d.get("cortexkit"))
                            .is_some()
                    {
                        out.push(value);
                    }
                    map.values().for_each(|v| compactions(v, out));
                }
                Value::Array(items) => items.iter().for_each(|v| compactions(v, out)),
                _ => {}
            }
        }
        let vectors = crate::vectors::load("display-pi.json");
        let mut found = Vec::new();
        compactions(&vectors, &mut found);
        assert!(!found.is_empty());
        for entry in found {
            let session = entry["details"]["cortexkit"]["session"].as_str().unwrap();
            assert_eq!(pi_marker_checkpoint(entry, session), None);
        }
    }
}
