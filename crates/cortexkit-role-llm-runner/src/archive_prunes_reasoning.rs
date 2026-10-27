//! The declared exception to byte-stable reads of an archived session.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `view_refused` reason a runner reports for a view that would put
/// pruned content back (CONTRACT §9.14.1), and the reason it records when a
/// post-prune Setup answer's `initial` would do the same.
pub const PRUNED_REASONING_RESTORED: &str = "pruned_reasoning_restored";

/// Pruning kinds are open strings on pages. An unknown kind means the part
/// stays in place but its changed payload cannot be trusted by that reader.
pub mod kinds {
    pub const REASONING: &str = "reasoning";
    pub const OPAQUE_REASONING: &str = "opaque_reasoning";
    pub const SIGNATURE: &str = "signature";
    pub const ALL: &[&str] = &[REASONING, OPAQUE_REASONING, SIGNATURE];
}

/// Settings required when `archive_prunes_reasoning` is declared.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ArchivePrunesReasoning {
    pub kinds: Vec<String>,
}

impl ArchivePrunesReasoning {
    pub fn new(kinds: Vec<String>) -> Self {
        Self { kinds }
    }

    /// Declarations list at least one known kind, with no duplicates.
    pub fn valid(&self) -> bool {
        !self.kinds.is_empty()
            && self.kinds.iter().enumerate().all(|(index, kind)| {
                kinds::ALL.contains(&kind.as_str()) && !self.kinds[..index].contains(kind)
            })
    }
}

/// One pruned part served on a page, repeated on every read of that part.
/// Identity and the digest describe the original write, not the empty payload.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PrunedReasoning {
    pub mid: String,
    pub part: String,
    pub kind: String,
    /// Length of the original part's RFC 8785 (JCS) encoding.
    pub bytes: u64,
    /// The original part's write-time `part_digests` digest.
    pub sha256: String,
}

/// A session's prune state, on `session.head`, `session.read` pages, Setup
/// requests and advance notices of a runner that declares
/// `archive_prunes_reasoning`. Absent until the session's first prune, so
/// "never pruned" needs no stored value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ReasoningPruned {
    /// 1 after the first prune, raised by one in the same commit as every
    /// later prune. Never 0.
    pub generation: u64,
    /// The highest ordinal any prune of the session has touched.
    pub through_ordinal: u64,
    /// When the latest prune committed, in milliseconds since the Unix
    /// epoch. Display only: no party compares it with its own clock.
    pub at_ms: u64,
}

impl ReasoningPruned {
    pub fn new(generation: u64, through_ordinal: u64, at_ms: u64) -> Self {
        Self {
            generation,
            through_ordinal,
            at_ms,
        }
    }

    /// Whether the value can describe a prune: the generation counts from 1.
    pub fn valid(&self) -> bool {
        self.generation >= 1
    }
}

/// One part the runner pruned, as its own stored prune state records it:
/// where the part is and which kind removed what. The runner checks views
/// against this record, not against digests, because part digests keep
/// their write-time values after a prune.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PrunedPartAt {
    pub ordinal: u64,
    pub part: String,
    pub kind: String,
}

impl PrunedPartAt {
    pub fn new(ordinal: u64, part: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            ordinal,
            part: part.into(),
            kind: kind.into(),
        }
    }
}

fn non_empty_string(value: Option<&Value>) -> bool {
    value.and_then(Value::as_str).is_some_and(|s| !s.is_empty())
}

/// Whether `part`, a message part in the stored formats CONTRACT §9.14.1
/// names, carries content that a prune of `kind` removes, so sending it
/// would put that content back:
///
/// - `reasoning`: a `reasoning` part with non-empty `text` or any
///   `signature`, or a `redacted_reasoning` part with non-empty `data`;
/// - `opaque_reasoning`: a `redacted_reasoning` part whose item JSON in
///   `data` still has `encrypted_content`; its readable summary may be sent;
/// - `signature`: a non-reasoning part with a non-empty `signature` or
///   `thought_signature`, or a non-empty `provider_state.signature`.
///
/// A kind this crate does not define is `false`: a runner applies only the
/// kinds it declares, and checks each with its own rule.
pub fn carries_pruned_content(kind: &str, part: &Value) -> bool {
    let kind_of = part.get("type").and_then(Value::as_str).unwrap_or_default();
    match kind {
        kinds::REASONING => match kind_of {
            "reasoning" => non_empty_string(part.get("text")) || part.get("signature").is_some(),
            "redacted_reasoning" => non_empty_string(part.get("data")),
            _ => false,
        },
        kinds::OPAQUE_REASONING => {
            kind_of == "redacted_reasoning"
                && part
                    .get("data")
                    .and_then(Value::as_str)
                    .and_then(|data| serde_json::from_str::<Value>(data).ok())
                    .is_some_and(|item| item.get("encrypted_content").is_some())
        }
        kinds::SIGNATURE => {
            !matches!(kind_of, "reasoning" | "redacted_reasoning")
                && (non_empty_string(part.get("signature"))
                    || non_empty_string(part.get("thought_signature"))
                    || non_empty_string(
                        part.get("provider_state").and_then(|s| s.get("signature")),
                    ))
        }
        _ => false,
    }
}

/// The pruned part a replacement would restore, if any (CONTRACT §9.14.1).
///
/// `stands_for` is the half-open ordinal range the replacement stands for:
/// a view `replace` range's `[from, to)`, widened to start at 0 for the
/// first range of a lineage whose `first_ordinal` is above 0, because that
/// range is the view's head and carries everything before the lineage. A
/// post-prune Setup answer's `initial` stands for the same as its range,
/// widened the same way. `messages` is the replacement's JSON array of
/// messages, each with a `content` array of parts. Pruned parts are taken in
/// the order given; the first whose ordinal lies in `stands_for` and whose
/// kind's removed content appears in any replacement part is returned.
pub fn restored_part<'a>(
    pruned: &'a [PrunedPartAt],
    stands_for: std::ops::Range<u64>,
    messages: &Value,
) -> Option<&'a PrunedPartAt> {
    let parts: Vec<&Value> = messages
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|message| message.get("content").and_then(Value::as_array))
        .flatten()
        .collect();
    pruned.iter().find(|entry| {
        stands_for.contains(&entry.ordinal)
            && parts
                .iter()
                .any(|part| carries_pruned_content(&entry.kind, part))
    })
}

/// The pruned part a keep-range part edit would restore, if any (CONTRACT
/// §9.14.1). `ordinal` and `part` are the edit's; `bytes` is the part the
/// edit sends, decoded from its `bytes`. A `drop` edit sends nothing and
/// restores nothing.
///
/// The check is per message, as for a replacement: an edit at an ordinal
/// holding a part pruned with some kind restores it when its bytes carry
/// that kind's removed content, whichever part of the message it edits, so
/// moving the content into a sibling part does not get it past. The pruned
/// entry for the edited part itself is returned first; otherwise the first
/// matching entry at that ordinal, in the order given.
pub fn restored_by_edit<'a>(
    pruned: &'a [PrunedPartAt],
    ordinal: u64,
    part: &str,
    bytes: &Value,
) -> Option<&'a PrunedPartAt> {
    let restores = |entry: &&PrunedPartAt| {
        entry.ordinal == ordinal && carries_pruned_content(&entry.kind, bytes)
    };
    pruned
        .iter()
        .filter(restores)
        .find(|entry| entry.part == part)
        .or_else(|| pruned.iter().find(restores))
}

impl PrunedReasoning {
    pub fn new(
        mid: impl Into<String>,
        part: impl Into<String>,
        kind: impl Into<String>,
        bytes: u64,
        sha256: impl Into<String>,
    ) -> Self {
        Self {
            mid: mid.into(),
            part: part.into(),
            kind: kind.into(),
            bytes,
            sha256: sha256.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        content_digest::content_digest,
        describe::{check_describe, RoleDescribe},
        display::DisplayPage,
        history::HistoryPage,
        read::{ModelPage, ReadPage},
    };
    use serde_json::{json, Value};

    #[test]
    fn archive_pruning_vectors_preserve_required_fields_and_write_time_digests() {
        let file = crate::vectors::load("archive-prunes-reasoning.json");
        for case in file["cases"].as_array().unwrap() {
            let page: ReadPage = serde_json::from_value(case["page"].clone()).unwrap();
            assert_eq!(serde_json::to_value(&page).unwrap(), case["page"]);
            let entry = &page.messages[0];
            let pruning = &page.pruned_reasoning[0];
            assert_eq!(pruning.sha256, content_digest(&case["original"]));
            assert_eq!(
                pruning.bytes,
                serde_jcs::to_vec(&case["original"]).unwrap().len() as u64
            );
            assert_eq!(
                entry.digest.as_ref().unwrap(),
                &content_digest(&case["original_message"])
            );
            assert_eq!(
                entry.part_digests.as_ref().unwrap()[0].digest,
                pruning.sha256
            );
            assert_eq!(entry.message["content"][0], case["served"]);
            assert_eq!(case["served"]["type"], case["original"]["type"]);
            for key in case["served"].as_object().unwrap().keys() {
                assert!(
                    case["original"].get(key).is_some(),
                    "no block member is added"
                );
            }
            match case["original"]["type"].as_str().unwrap() {
                "reasoning" => {
                    assert_eq!(case["served"]["text"], "");
                    assert!(case["served"].get("signature").is_none());
                }
                "redacted_reasoning" if case["kind"] == "reasoning" => {
                    assert_eq!(case["served"]["data"], "")
                }
                "redacted_reasoning" => {
                    let original: Value =
                        serde_json::from_str(case["original"]["data"].as_str().unwrap()).unwrap();
                    let served: Value =
                        serde_json::from_str(case["served"]["data"].as_str().unwrap()).unwrap();
                    assert_eq!(original["summary"], served["summary"]);
                    assert!(served.get("encrypted_content").is_none());
                }
                "tool_call" => {
                    assert_eq!(case["served"]["arguments"], case["original"]["arguments"]);
                    assert!(case["served"].get("thought_signature").is_none());
                }
                "text_with_state" => {
                    assert_eq!(case["served"]["text"], case["original"]["text"]);
                    assert_eq!(case["served"]["provider_state"]["signature"], "");
                }
                other => panic!("unexpected vector {other}"),
            }
        }
    }

    #[test]
    fn archive_pruning_all_page_types_and_unknown_kind_round_trip() {
        let row = PrunedReasoning::new("example", "m0#0", "future-kind", 11, "a".repeat(64));
        let pages = [
            serde_json::to_value(ReadPage::no_lineage()).unwrap(),
            serde_json::to_value(ModelPage::no_lineage()).unwrap(),
            serde_json::to_value(HistoryPage::no_lineage()).unwrap(),
        ];
        for page in pages {
            assert!(page.get("pruned_reasoning").is_none());
        }
        let history = HistoryPage::new("example", vec![])
            .with_history_version(1)
            .with_pruned_reasoning(vec![row.clone()]);
        let display = DisplayPage::from_history(&history, |_| {
            Ok::<_, ()>(crate::display::DisplayRow::new("assistant", vec![]))
        })
        .unwrap();
        assert_eq!(display.pruned_reasoning, vec![row.clone()]);
        let encoded = serde_json::to_value(&display).unwrap();
        assert_eq!(
            serde_json::from_value::<DisplayPage>(encoded).unwrap(),
            display
        );
        let model = ModelPage::new("example", vec![]).with_pruned_reasoning(vec![row]);
        assert_eq!(
            serde_json::from_value::<ModelPage>(serde_json::to_value(&model).unwrap()).unwrap(),
            model
        );
    }

    #[test]
    fn reasoning_pruned_head_and_page_vectors_round_trip_and_stay_absent_by_default() {
        let file = crate::vectors::load("reasoning-pruned.json");
        for case in crate::vectors::cases(&file, "heads") {
            let name = case["name"].as_str().unwrap();
            let head: crate::read::HeadMeta = crate::vectors::round_trip(name, &case["head"]);
            assert!(head.lineage_consistent(), "{name}");
            match name {
                "never_pruned" => assert_eq!(head.reasoning_pruned, None),
                _ => assert!(head.reasoning_pruned.unwrap().valid(), "{name}"),
            }
        }
        let generations: Vec<_> = crate::vectors::cases(&file, "heads")
            .iter()
            .filter_map(|c| c["head"]["reasoning_pruned"]["generation"].as_u64())
            .collect();
        assert_eq!(generations, [1, 2], "generation counts from 1");
        for case in crate::vectors::cases(&file, "pages") {
            let name = case["name"].as_str().unwrap();
            let state = match case["view"].as_str().unwrap() {
                "raw" => {
                    crate::vectors::round_trip::<ReadPage>(name, &case["page"]).reasoning_pruned
                }
                _ => crate::vectors::round_trip::<ModelPage>(name, &case["page"]).reasoning_pruned,
            };
            assert_eq!(state, Some(ReasoningPruned::new(2, 7, 1767312000000)));
        }
        for case in crate::vectors::cases(&file, "invalid") {
            let state: ReasoningPruned =
                serde_json::from_value(case["reasoning_pruned"].clone()).unwrap();
            assert!(!state.valid(), "{}", case["name"]);
        }
        // Existing heads and pages keep their bytes: the member is omitted
        // when absent, never encoded as null.
        for encoded in [
            serde_json::to_value(crate::read::HeadMeta::new("example", 0, 1)).unwrap(),
            serde_json::to_value(ReadPage::new("example", vec![])).unwrap(),
            serde_json::to_value(ModelPage::new("example", vec![])).unwrap(),
        ] {
            assert!(encoded.get("reasoning_pruned").is_none());
        }
        let mut never_written = crate::read::HeadMeta::no_lineage();
        assert!(never_written.lineage_consistent());
        never_written = never_written.with_reasoning_pruned(ReasoningPruned::new(1, 0, 1));
        assert!(
            !never_written.lineage_consistent(),
            "a session never written was never pruned"
        );
    }

    #[test]
    fn restored_part_follows_each_recorded_kind() {
        let file = crate::vectors::load("reasoning-pruned.json");
        let mut kinds_seen = std::collections::BTreeSet::new();
        for case in crate::vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let pruned: Vec<PrunedPartAt> = case["pruned"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    PrunedPartAt::new(
                        p["ordinal"].as_u64().unwrap(),
                        p["part"].as_str().unwrap(),
                        p["kind"].as_str().unwrap(),
                    )
                })
                .collect();
            let from = case["stands_for"]["from"].as_u64().unwrap();
            let to = case["stands_for"]["to"].as_u64().unwrap();
            if let Some(range) = case.get("range") {
                // The head range of a cut lineage stands for everything
                // before the lineage as well as its own ordinals.
                let first = case["first_ordinal"].as_u64().unwrap();
                assert_eq!(range["from"].as_u64(), Some(first), "{name}");
                assert_eq!((from, to), (0, range["to"].as_u64().unwrap()), "{name}");
            }
            let messages: Value = serde_json::from_str(case["bytes"].as_str().unwrap()).unwrap();
            let found = restored_part(&pruned, from..to, &messages)
                .map(|p| json!({"ordinal": p.ordinal, "part": p.part}));
            assert_eq!(found.unwrap_or(Value::Null), case["restored"], "{name}");
            if !case["restored"].is_null() {
                kinds_seen.insert(pruned[0].kind.clone());
            }
        }
        assert_eq!(
            kinds_seen.iter().map(String::as_str).collect::<Vec<_>>(),
            {
                let mut all = kinds::ALL.to_vec();
                all.sort();
                all
            },
            "one refusal vector per kind"
        );
        // The served (pruned) form of every archive-pruning vector carries
        // nothing its kind removes, while the original does.
        let pruning = crate::vectors::load("archive-prunes-reasoning.json");
        for case in pruning["cases"].as_array().unwrap() {
            let kind = case["kind"].as_str().unwrap();
            assert!(carries_pruned_content(kind, &case["original"]), "{kind}");
            assert!(!carries_pruned_content(kind, &case["served"]), "{kind}");
        }
        assert!(!carries_pruned_content(
            "future-kind",
            &json!({"type":"reasoning","text":"x"})
        ));
    }

    #[test]
    fn restored_by_edit_follows_each_recorded_kind() {
        let file = crate::vectors::load("reasoning-pruned.json");
        let mut kinds_refused = std::collections::BTreeSet::new();
        let mut kinds_allowed = std::collections::BTreeSet::new();
        for case in crate::vectors::cases(&file, "edit_refusals") {
            let name = case["name"].as_str().unwrap();
            let pruned: Vec<PrunedPartAt> = case["pruned"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| {
                    PrunedPartAt::new(
                        p["ordinal"].as_u64().unwrap(),
                        p["part"].as_str().unwrap(),
                        p["kind"].as_str().unwrap(),
                    )
                })
                .collect();
            let edit = &case["edit"];
            // A drop sends no part, so there is nothing to check.
            let found = edit["bytes"].as_str().and_then(|bytes| {
                let part: Value = serde_json::from_str(bytes).unwrap();
                restored_by_edit(
                    &pruned,
                    edit["ordinal"].as_u64().unwrap(),
                    edit["part"].as_str().unwrap(),
                    &part,
                )
                .map(|p| json!({"ordinal": p.ordinal, "part": p.part}))
            });
            assert_eq!(found.unwrap_or(Value::Null), case["restored"], "{name}");
            if case["restored"].is_null() {
                kinds_allowed.insert(pruned[0].kind.clone());
            } else {
                kinds_refused.insert(pruned[0].kind.clone());
            }
        }
        let mut all = kinds::ALL.to_vec();
        all.sort();
        for seen in [kinds_refused, kinds_allowed] {
            assert_eq!(
                seen.iter().map(String::as_str).collect::<Vec<_>>(),
                all,
                "a refused and an allowed edit vector per kind"
            );
        }
    }

    #[test]
    fn archive_pruning_describe_is_paired_and_strict() {
        let base = RoleDescribe::new(
            vec![crate::describe::Major::new(
                crate::PROVIDES,
                vec![
                    "role.describe".into(),
                    "session.baseline".into(),
                    "session.read".into(),
                ],
                "alpha",
            )],
            "example",
        )
        .with_session_capabilities_from("baseline");
        let raw = serde_json::to_value(base).unwrap();
        assert!(check_describe(&raw).is_ok());
        for kinds in [
            json!([]),
            json!(["unknown"]),
            json!(["reasoning", "reasoning"]),
        ] {
            let mut bad = raw.clone();
            bad["capabilities"] = json!(["archive_prunes_reasoning"]);
            bad["archive_prunes_reasoning"] = json!({"kinds": kinds});
            assert!(check_describe(&bad).is_err());
        }
        let mut paired = raw.clone();
        paired["archive_prunes_reasoning"] = json!({"kinds": kinds::ALL});
        assert!(check_describe(&paired).is_err());
        paired["capabilities"] = json!(["archive_prunes_reasoning"]);
        assert!(check_describe(&paired).is_ok());
        paired
            .as_object_mut()
            .unwrap()
            .remove("archive_prunes_reasoning");
        assert!(check_describe(&paired).is_err());
    }
}
