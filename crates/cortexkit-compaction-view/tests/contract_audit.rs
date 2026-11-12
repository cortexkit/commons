//! Regression cases for compaction-contract behavior that failed before its fix.

use cortexkit_compaction_view::render::{apply_edits, plan, PlanItem};
use cortexkit_role_compaction::part::{Part, PartBody};
use cortexkit_role_compaction::read::{entry_digest, Entry, NoticeOrigin, Role};
use cortexkit_role_compaction::view::{Edit, EditOp, InsertAt, InsertedMessage, ViewRefusal};
use serde_json::json;

/// Keeping only a result's image attachment changes its label in the render
/// plan from `[file]` to `[image]`, as the neutral rendering shows it, so a
/// runner rendering from the plan and the provider planning against the
/// neutral form see the same text (W4.7, W6.3).
#[test]
fn keeping_some_attachments_updates_the_render_plan_label() {
    let entry: Entry = serde_json::from_value(json!({
        "ordinal": 1, "mid": "call", "role": "assistant", "written_at": 1,
        "digest": "entry", "parts": [{
            "id": "tool", "tag": 1, "digest": "part", "kind": "tool",
            "call_id": "c", "name": "read", "input": {}, "status": "completed",
            "output": "", "output_bytes": 0,
            "attachments": [
                {"id": "image", "kind": "file", "mime": "image/png", "transport": "inline"},
                {"id": "pdf", "kind": "file", "mime": "application/pdf", "transport": "inline"}
            ]
        }]
    }))
    .unwrap();
    let edits = [Edit::new(
        1,
        EditOp::Replace {
            mid: "call".into(),
            part: "tool".into(),
            field: "attachments".into(),
            value: json!(["image"]),
            expect: "part".into(),
        },
    )];
    let rendered = apply_edits(std::slice::from_ref(&entry), &edits).unwrap();
    assert_eq!(
        rendered[0].wire_text(&rendered[0].parts[0]).as_deref(),
        Some("§1§ [image]")
    );
    let planned = plan(&[entry], &edits).unwrap();
    let PlanItem::Entry { parts, .. } = &planned.items[0] else {
        panic!("expected an entry");
    };
    assert_eq!(parts[0].empty_result_label.as_deref(), Some("[image]"));
}

/// Tag notation is syntactic (W6.4): digits too many for a number still
/// make an inserted text tagged.
#[test]
fn an_insert_with_tag_notation_too_long_for_a_number_is_refused() {
    let edits = [Edit::new(
        1,
        EditOp::Insert {
            at: InsertAt::Start,
            message: InsertedMessage::new(
                "head",
                Role::User,
                vec!["quoted §18446744073709551616§ prefix".into()],
            ),
        },
    )];
    assert_eq!(
        plan(&[], &edits),
        Err(ViewRefusal::TaggedInsert { id: "head".into() })
    );
}

/// The JSON numbering function returns entries whose checksums cover the
/// numbers it gave their parts (W7).
#[test]
fn numbering_returns_entries_with_checksums_over_their_numbers() {
    let parts = vec![Part::new(
        "text",
        PartBody::Text {
            text: "hello".into(),
        },
    )];
    let notice = NoticeOrigin::new("audit");
    let mut entry = Entry::new(1, "m", Role::User, 1, parts);
    entry.digest = entry_digest(Role::User, Some(&notice), &entry.parts);
    entry.notice = Some(notice);
    let output = cortexkit_compaction_view::vectors::call(
        "assign_tags",
        json!({"checkpoint": 0, "entries": [entry]}),
    );
    let numbered: Vec<Entry> = serde_json::from_value(output["ok"].clone()).unwrap();
    assert_eq!(numbered[0].parts[0].tag, Some(1));
    assert_eq!(
        numbered[0].digest,
        entry_digest(
            numbered[0].role.clone(),
            numbered[0].notice.as_ref(),
            &numbered[0].parts
        )
    );
}

/// Every tool part in this crate's vector file encodes to the bytes the
/// contract crate wrote before a tool part boxed its attachments, approval
/// and delivered text, and reads those fields back. The fixture holds each
/// distinct tool part of `vectors/cases.json` with the encoding
/// cortexkit-role-compaction 0.3.5 wrote for it.
#[test]
fn every_vector_tool_part_keeps_its_bytes() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/unboxed_tool_parts.jsonl");
    let fixture = std::fs::read_to_string(fixture).unwrap();
    assert!(
        fixture.lines().count() >= 9,
        "the fixture covers the vectors"
    );
    for line in fixture.lines() {
        let case: serde_json::Value = serde_json::from_str(line).unwrap();
        let unforked = case["unforked"].as_str().unwrap();
        let part: Part = serde_json::from_value(case["wire"].clone()).unwrap();
        assert_eq!(serde_json::to_string(&part).unwrap(), unforked);
        let written: serde_json::Value = serde_json::from_str(unforked).unwrap();
        let PartBody::Tool(tool) = &part.body else {
            panic!("a tool part decodes as one");
        };
        assert_eq!(
            !tool.attachments().is_empty(),
            written.get("attachments").is_some()
        );
        assert_eq!(tool.approval().is_some(), written.get("approval").is_some());
        assert_eq!(
            tool.delivered().is_some(),
            written.get("delivered").is_some()
        );
    }
}
