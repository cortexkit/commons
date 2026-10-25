//! The declared exception to byte-stable reads of an archived session.

use serde::{Deserialize, Serialize};

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
