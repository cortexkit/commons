//! Tests for applying views and checking provider rules.

use cortexkit_role_compaction::part::{
    Attachment, AttachmentItem, Delivered, FileMedia, OutputFormat, Part, PartBody, Seal, ToolPart,
    ToolStatus,
};
use cortexkit_role_compaction::read::{Entry, Role};
use cortexkit_role_compaction::rules::{EditCost, ProviderRules, RuleBreak, SignedParts};
use cortexkit_role_compaction::view::{
    Edit, EditOp, InsertAt, InsertedImage, InsertedMessage, View, ViewRefusal,
};
use serde_json::json;

use crate::render::{
    apply, apply_edits, check_carried, cleared_ride, first_change, plan, FirstChange, RideRender,
    Source,
};
use crate::rules::{check, invalid_signed_parts, Floor};

fn entry(
    ordinal: u64,
    mid: &str,
    role: Role,
    parts: Vec<Part>,
    generated_under: Option<u64>,
) -> Entry {
    let entry = Entry::new(ordinal, mid, role, ordinal * 1_000, parts);
    match generated_under {
        Some(version) => entry.with_generated_under(View::seal(version, Vec::new()).applied()),
        None => entry,
    }
}

/// A part with a number and seal; its checksum covers the seal and body,
/// not its number.
fn mk(id: impl Into<String>, tag: Option<u64>, seal: Seal, body: PartBody) -> Part {
    let mut part = Part::new(id, body).with_seal(seal);
    part.tag = tag;
    part
}

fn text(id: &str, tag: u64, value: &str) -> Part {
    mk(
        id,
        Some(tag),
        Seal::None,
        PartBody::Text { text: value.into() },
    )
}

fn signed(id: &str) -> Part {
    mk(
        id,
        None,
        Seal::Signed,
        PartBody::Reasoning {
            text: Some("thinking".into()),
            text_bytes: Some(8),
        },
    )
}

fn strip_entries() -> Vec<Entry> {
    vec![
        entry(1, "anchor", Role::User, vec![text("t1", 1, "old")], None),
        entry(
            2,
            "reply",
            Role::Assistant,
            vec![signed("thinking"), tool("call", 2, ToolStatus::Completed)],
            None,
        ),
    ]
}

fn strip_view(anchor: Option<&str>, policy: Option<&str>) -> View {
    View::seal_with_policy(
        1,
        vec![insert(1, InsertAt::Start, "summary")],
        anchor.map(str::to_owned),
        policy.map(str::to_owned),
    )
}

fn assert_policy_vector(name: &str) {
    let vectors: Vec<crate::vectors::Vector> =
        serde_json::from_str(include_str!("../vectors/cases.json")).unwrap();
    let vector = vectors.iter().find(|vector| vector.name == name).unwrap();
    assert_eq!(
        crate::vectors::call(&vector.function, vector.input.clone()),
        vector.output,
        "{name}"
    );
}

#[test]
fn strip_vector_drops_newer_thinking_and_keeps_the_final_tool() {
    assert_policy_vector(
        "strip an invalidated signed part newer than the anchor and keep the newest assistant tool",
    );
    let entries = strip_entries();
    let view = strip_view(Some("anchor"), Some("strip"));
    let planned = crate::render::plan_view(&entries, &view, &bound(), Floor::none()).unwrap();
    let crate::render::PlanItem::Entry { mid, parts, .. } = planned.items.last().unwrap() else {
        panic!("the reply must remain");
    };
    assert_eq!(mid, "reply");
    assert_eq!(
        parts
            .iter()
            .map(|part| part.id.as_str())
            .collect::<Vec<_>>(),
        ["call"]
    );
    assert!(parts[0].replace.is_empty());
    assert_eq!(parts[0].tag, Some(2));
    assert!(view.verify());
    assert_eq!(view.edits.len(), 1, "temporary drops never change the view");
    let rendered =
        crate::render::apply_with_rules(&entries, &view, &bound(), Floor::none()).unwrap();
    assert_eq!(
        rendered.last().unwrap().parts,
        vec![entries[1].parts[1].clone()]
    );
    assert!(
        check(&entries, &view.edits, &bound(), Floor::none()).is_err(),
        "the edit-only checker remains strict"
    );
}

#[test]
fn anchor_vector_refuses_a_signature_at_the_anchor() {
    assert_policy_vector("a signed part at the anchor is never stripped");
    let view = strip_view(Some("reply"), Some("strip"));
    assert_eq!(
        crate::render::plan_view(&strip_entries(), &view, &bound(), Floor::none()),
        Err(ViewRefusal::Rule(RuleBreak::SignedPartInvalid {
            mid: "reply".into(),
            part: "thinking".into(),
        })),
    );
}

#[test]
fn stripping_requires_a_known_policy_and_an_active_earlier_anchor() {
    for (anchor, policy) in [
        (None, Some("strip")),
        (Some("removed-branch"), Some("strip")),
        (Some("anchor"), None),
        (Some("anchor"), Some("future-policy")),
    ] {
        assert!(
            matches!(
                crate::render::plan_view(
                    &strip_entries(),
                    &strip_view(anchor, policy),
                    &bound(),
                    Floor::none(),
                ),
                Err(ViewRefusal::Rule(RuleBreak::SignedPartInvalid { .. }))
            ),
            "{anchor:?} {policy:?}"
        );
    }
    let mut entries = strip_entries();
    let view = strip_view(Some("anchor"), Some("strip"));
    entries[0].mid = "another-message-at-the-same-ordinal".into();
    assert!(crate::render::plan_view(&entries, &view, &bound(), Floor::none()).is_err());
}

#[test]
fn stripping_never_drops_a_protected_last_part_or_tool_or_encrypted_part() {
    let view = strip_view(Some("anchor"), Some("strip"));
    let mut entries = strip_entries();
    entries[1].parts.pop();
    assert!(crate::render::plan_view(&entries, &view, &bound(), Floor::none()).is_err());
    entries = strip_entries();
    entries[1].parts[1].seal = Seal::Signed;
    assert!(crate::render::plan_view(&entries, &view, &bound(), Floor::none()).is_err());
    entries = strip_entries();
    entries[1].parts[0].seal = Seal::Encrypted;
    let rendered =
        crate::render::apply_with_rules(&entries, &view, &bound(), Floor::none()).unwrap();
    assert_eq!(rendered.last().unwrap().parts, entries[1].parts);
}

fn tool_body(id: &str, status: ToolStatus) -> ToolPart {
    let running = status == ToolStatus::Running;
    let body = ToolPart::new(
        format!("call-{id}"),
        "edit",
        json!({"path": "a.rs", "old_string": "x".repeat(100), "count": 3}),
        status,
    )
    .with_attachments(vec![Attachment::new(
        "img-1",
        AttachmentItem::File(FileMedia::inline("image/png", Some(2048))),
    )]);
    if running {
        body
    } else {
        body.with_output("done").with_output_bytes(4)
    }
}

fn tool(id: &str, tag: u64, status: ToolStatus) -> Part {
    mk(
        id,
        Some(tag),
        Seal::None,
        PartBody::Tool(tool_body(id, status)),
    )
}

fn drop_part(since: u64, mid: &str, part: &str) -> Edit {
    Edit::new(
        since,
        EditOp::Drop {
            mid: mid.into(),
            part: Some(part.into()),
        },
    )
}

fn drop_message(since: u64, mid: &str) -> Edit {
    Edit::new(
        since,
        EditOp::Drop {
            mid: mid.into(),
            part: None,
        },
    )
}

fn replace(
    since: u64,
    entries: &[Entry],
    mid: &str,
    part: &str,
    field: &str,
    value: serde_json::Value,
) -> Edit {
    let expect = entries
        .iter()
        .find(|entry| entry.mid == mid)
        .and_then(|entry| entry.parts.iter().find(|candidate| candidate.id == part))
        .map(|found| found.digest.clone())
        .unwrap();
    Edit::new(
        since,
        EditOp::Replace {
            mid: mid.into(),
            part: part.into(),
            field: field.into(),
            value,
            expect,
        },
    )
}

fn insert(since: u64, at: InsertAt, id: &str) -> Edit {
    Edit::new(
        since,
        EditOp::Insert {
            at,
            message: InsertedMessage::new(
                id,
                Role::User,
                vec!["<session-history>...</session-history>".into()],
            ),
        },
    )
}

fn transcript() -> Vec<Entry> {
    vec![
        entry(0, "u1", Role::User, vec![text("t1", 1, "please fix")], None),
        entry(
            1,
            "a1",
            Role::Assistant,
            vec![
                signed("r1"),
                text("t2", 2, "looking"),
                tool("k1", 3, ToolStatus::Completed),
            ],
            None,
        ),
        entry(2, "u2", Role::User, vec![text("t3", 4, "go on")], None),
        entry(
            3,
            "a2",
            Role::Assistant,
            vec![signed("r2"), tool("k2", 5, ToolStatus::Running)],
            None,
        ),
    ]
}

fn bound() -> ProviderRules {
    ProviderRules::new(SignedParts::Bound, EditCost::Suffix)
}

#[test]
fn an_empty_view_renders_the_transcript_unchanged() {
    let entries = transcript();
    let rendered = apply(&entries, &View::seal(1, vec![])).unwrap();
    assert_eq!(rendered.len(), entries.len());
    for (message, entry) in rendered.iter().zip(&entries) {
        assert_eq!(message.parts, entry.parts);
    }
}

#[test]
fn drops_remove_parts_and_a_message_left_empty_disappears() {
    let entries = transcript();
    let edits = vec![drop_part(1, "a1", "k1"), drop_part(1, "u2", "t3")];
    let rendered = apply_edits(&entries, &edits).unwrap();
    let mids: Vec<_> = rendered
        .iter()
        .map(|message| match &message.source {
            Source::Entry { mid, .. } => mid.as_str(),
            Source::Inserted { id } => id.as_str(),
        })
        .collect();
    assert_eq!(mids, ["u1", "a1", "a2"]);
    let a1: Vec<_> = rendered[1]
        .parts
        .iter()
        .map(|part| part.id.as_str())
        .collect();
    assert_eq!(a1, ["r1", "t2"]);

    let rendered = apply_edits(&entries, &[drop_message(1, "a1")]).unwrap();
    assert_eq!(rendered.len(), 3);
}

#[test]
fn replaces_change_one_field_and_keep_id_and_tag() {
    let entries = transcript();
    let edits = vec![
        replace(1, &entries, "u1", "t1", "text", json!("[dropped §1§]")),
        replace(1, &entries, "a1", "k1", "output", json!("[dropped §3§]")),
        replace(1, &entries, "a1", "k1", "input.old_string", json!("xxx…")),
        replace(1, &entries, "a1", "k1", "attachments", json!([])),
    ];
    let rendered = apply_edits(&entries, &edits).unwrap();
    let PartBody::Text { text } = &rendered[0].parts[0].body else {
        panic!()
    };
    assert_eq!(text, "[dropped §1§]");
    let replaced = &rendered[1].parts[2];
    assert_eq!((replaced.id.as_str(), replaced.tag), ("k1", Some(3)));
    let PartBody::Tool(tool) = &replaced.body else {
        panic!()
    };
    assert_eq!(tool.output.as_deref(), Some("[dropped §3§]"));
    assert_eq!(tool.input.get("old_string").unwrap(), "xxx…");
    assert_eq!(tool.input.get("path").unwrap(), "a.rs");
    assert!(tool.attachments().is_empty());
    assert_ne!(replaced.digest, entries[1].parts[2].digest);
}

#[test]
fn bad_edits_are_refused_by_name() {
    let entries = transcript();
    let stale = Edit::new(
        1,
        EditOp::Replace {
            mid: "u1".into(),
            part: "t1".into(),
            field: "text".into(),
            value: json!("x"),
            expect: "old".into(),
        },
    );
    assert!(matches!(
        apply_edits(&entries, &[stale]),
        Err(ViewRefusal::StalePart { .. })
    ));
    for edit in [
        replace(1, &entries, "a1", "r1", "text", json!("x")),
        replace(1, &entries, "a1", "k1", "input.count", json!("x")),
        replace(1, &entries, "a1", "k1", "input.missing", json!("x")),
        replace(1, &entries, "a1", "k1", "attachments", json!(["img-2"])),
        replace(1, &entries, "u1", "t1", "text", json!(7)),
    ] {
        assert!(
            matches!(
                apply_edits(&entries, std::slice::from_ref(&edit)),
                Err(ViewRefusal::BadField { .. })
            ),
            "{edit:?}"
        );
    }
    assert!(matches!(
        apply_edits(&entries, &[drop_part(1, "a2", "k2")]),
        Err(ViewRefusal::RunningTool { .. })
    ));
    assert!(matches!(
        apply_edits(&entries, &[drop_message(1, "a2")]),
        Err(ViewRefusal::RunningTool { .. })
    ));
    assert!(matches!(
        apply_edits(&entries, &[drop_part(1, "nope", "t1")]),
        Err(ViewRefusal::UnknownMessage { .. })
    ));
    assert!(matches!(
        apply_edits(&entries, &[drop_part(1, "u1", "nope")]),
        Err(ViewRefusal::UnknownPart { .. })
    ));
    assert!(matches!(
        apply_edits(
            &entries,
            &[
                drop_part(1, "u1", "t1"),
                replace(1, &entries, "u1", "t1", "text", json!("x"))
            ]
        ),
        Err(ViewRefusal::Conflict { .. })
    ));
    assert!(matches!(
        apply_edits(
            &entries,
            &[
                replace(1, &entries, "u1", "t1", "text", json!("x")),
                replace(1, &entries, "u1", "t1", "text", json!("y"))
            ]
        ),
        Err(ViewRefusal::Conflict { .. })
    ));
    assert!(matches!(
        apply_edits(&entries, &[insert(1, InsertAt::Before("nope".into()), "h")]),
        Err(ViewRefusal::UnknownAnchor { .. })
    ));
    assert!(matches!(
        apply_edits(
            &entries,
            &[
                insert(1, InsertAt::Start, "h"),
                insert(1, InsertAt::Start, "h")
            ]
        ),
        Err(ViewRefusal::DuplicateInsert { .. })
    ));
}

#[test]
fn inserts_render_where_they_are_anchored_even_before_a_dropped_message() {
    let entries = transcript();
    let edits = vec![
        insert(1, InsertAt::Start, "m0"),
        drop_message(1, "u1"),
        drop_message(1, "a1"),
        insert(1, InsertAt::Before("u2".into()), "m1"),
    ];
    let rendered = apply_edits(&entries, &edits).unwrap();
    let sources: Vec<_> = rendered
        .iter()
        .map(|message| message.source.clone())
        .collect();
    assert_eq!(
        sources,
        [
            Source::Inserted { id: "m0".into() },
            Source::Inserted { id: "m1".into() },
            Source::Entry {
                ordinal: 2,
                mid: "u2".into()
            },
            Source::Entry {
                ordinal: 3,
                mid: "a2".into()
            },
        ]
    );
    assert_eq!(rendered[0].parts[0].tag, None);
}

#[test]
fn a_view_with_a_wrong_checksum_is_refused() {
    let entries = transcript();
    let mut view = View::seal(2, vec![drop_part(2, "u1", "t1")]);
    assert!(apply(&entries, &view).is_ok());
    view.edits.clear();
    assert!(matches!(
        apply(&entries, &view),
        Err(ViewRefusal::BadChecksum)
    ));
}

#[test]
fn the_wire_shape_puts_since_and_op_side_by_side() {
    let edit = drop_part(3, "u1", "t1");
    assert_eq!(
        serde_json::to_value(&edit).unwrap(),
        json!({"since": 3, "op": "drop", "mid": "u1", "part": "t1"})
    );
    let view = View::seal(
        3,
        vec![edit, insert(3, InsertAt::Before("u2".into()), "m1")],
    );
    let back: View = serde_json::from_value(serde_json::to_value(&view).unwrap()).unwrap();
    assert_eq!(back, view);
    assert!(back.verify());
}

#[test]
fn since_values_must_match_the_previous_view() {
    let carried = drop_part(2, "u1", "t1");
    let previous = View::seal(2, vec![carried.clone()]);
    // Carried unchanged, plus new edits from a version the runner skipped and
    // from the view's own version.
    let next = View::seal(
        5,
        vec![carried, drop_part(4, "a1", "k1"), drop_part(5, "u2", "t3")],
    );
    assert!(check_carried(Some(&previous), &next).is_ok());
    // A new edit that claims to be as old as the previous view.
    let backdated = View::seal(5, vec![drop_part(2, "a1", "k1")]);
    assert_eq!(
        check_carried(Some(&previous), &backdated),
        Err(ViewRefusal::BadSince)
    );
    // An edit from the future.
    let future = View::seal(5, vec![drop_part(6, "a1", "k1")]);
    assert_eq!(
        check_carried(Some(&previous), &future),
        Err(ViewRefusal::BadSince)
    );
    assert_eq!(
        check_carried(None, &View::seal(1, vec![drop_part(0, "a1", "k1")])),
        Err(ViewRefusal::BadSince)
    );
}

/// u1 | a1: r1 (signed), t2, k1 | u2 | a3: r3 (signed), t5 | u4 | a5: r5 (signed), t7
fn signed_transcript(generated: [u64; 3]) -> Vec<Entry> {
    vec![
        entry(0, "u1", Role::User, vec![text("t1", 1, "start")], None),
        entry(
            1,
            "a1",
            Role::Assistant,
            vec![
                signed("r1"),
                text("t2", 2, "one"),
                tool("k1", 3, ToolStatus::Completed),
            ],
            Some(generated[0]),
        ),
        entry(2, "u2", Role::User, vec![text("t4", 4, "more")], None),
        entry(
            3,
            "a3",
            Role::Assistant,
            vec![signed("r3"), text("t5", 5, "two")],
            Some(generated[1]),
        ),
        entry(4, "u4", Role::User, vec![text("t6", 6, "more")], None),
        entry(
            5,
            "a5",
            Role::Assistant,
            vec![signed("r5"), text("t7", 7, "three")],
            Some(generated[2]),
        ),
    ]
}

fn invalid(entries: &[Entry], edits: &[Edit]) -> Vec<String> {
    invalid_signed_parts(entries, edits, &bound(), Floor::none())
        .into_iter()
        .map(|(_, part)| part)
        .collect()
}

#[test]
fn an_edit_before_a_signed_part_invalidates_it_only_if_it_came_later() {
    let entries = signed_transcript([0, 0, 0]);
    let edits = vec![drop_part(1, "a1", "k1")];
    assert_eq!(invalid(&entries, &edits), ["r3", "r5"]);
    assert_eq!(
        check(&entries, &edits, &bound(), Floor::none()),
        Err(RuleBreak::SignedPartInvalid {
            mid: "a3".into(),
            part: "r3".into()
        })
    );

    // r5 was generated under view 1, which already held that drop.
    let entries = signed_transcript([0, 0, 1]);
    assert_eq!(invalid(&entries, &edits), ["r3"]);

    // An edit after every signed part touches none of them.
    let entries = signed_transcript([0, 0, 0]);
    assert!(invalid(&entries, &[drop_part(1, "a5", "t7")]).is_empty());

    // The same rules with no signature binding never object.
    let free = ProviderRules::new(SignedParts::None, EditCost::Suffix);
    assert!(check(&entries, &edits, &free, Floor::none()).is_ok());
}

#[test]
fn an_insert_right_before_a_message_invalidates_its_own_signed_part() {
    let entries = signed_transcript([0, 0, 0]);
    let edits = vec![insert(1, InsertAt::Before("a3".into()), "note")];
    assert_eq!(invalid(&entries, &edits), ["r3", "r5"]);
}

#[test]
fn a_new_history_head_at_the_start_invalidates_every_older_signed_part() {
    let entries = signed_transcript([1, 1, 2]);
    let edits = vec![insert(2, InsertAt::Start, "m0")];
    assert_eq!(invalid(&entries, &edits), ["r1", "r3"]);
}

#[test]
fn signed_parts_may_be_left_out_only_at_the_ends() {
    let entries = signed_transcript([0, 0, 0]);
    // Leaving out the oldest is fine.
    assert!(invalid(&entries, &[drop_part(1, "a1", "r1")]).is_empty());
    // Leaving out the oldest two is fine.
    assert!(invalid(
        &entries,
        &[drop_part(1, "a1", "r1"), drop_part(1, "a3", "r3")]
    )
    .is_empty());
    // Leaving out the middle one invalidates the newer one.
    assert_eq!(invalid(&entries, &[drop_part(1, "a3", "r3")]), ["r5"]);
    // Leaving out the newest keeps the older ones valid, alone or together
    // with the oldest.
    assert!(invalid(&entries, &[drop_part(1, "a5", "r5")]).is_empty());
    assert!(invalid(
        &entries,
        &[drop_part(1, "a1", "r1"), drop_part(1, "a5", "r5")]
    )
    .is_empty());
    // A gap made before r5 was generated was already part of its history.
    let entries = signed_transcript([0, 0, 1]);
    assert!(invalid(&entries, &[drop_part(1, "a3", "r3")]).is_empty());
}

fn invalid_under_floor(entries: &[Entry], edits: &[Edit], floor: &str) -> Vec<String> {
    invalid_signed_parts(entries, edits, &bound(), Floor::through(floor))
        .into_iter()
        .map(|(_, part)| part)
        .collect()
}

#[test]
fn a_signed_floor_covers_the_signed_parts_at_or_before_it() {
    let entries = signed_transcript([0, 0, 0]);
    let head = vec![insert(1, InsertAt::Start, "m0")];
    assert_eq!(invalid(&entries, &head), ["r1", "r3", "r5"]);
    // The runner strips r1 and r3, so the head can invalidate only r5.
    assert_eq!(invalid_under_floor(&entries, &head, "a3"), ["r5"]);
    // A floor at the newest message covers every signed part.
    assert!(invalid_under_floor(&entries, &head, "a5").is_empty());
    assert_eq!(
        check(&entries, &head, &bound(), Floor::through("a5")),
        Ok(())
    );
}

#[test]
fn an_edit_before_the_floor_still_invalidates_signed_parts_after_it() {
    let entries = signed_transcript([0, 0, 0]);
    let edits = vec![drop_part(1, "a1", "k1")];
    assert_eq!(invalid_under_floor(&entries, &edits, "a1"), ["r3", "r5"]);
    assert_eq!(
        check(&entries, &edits, &bound(), Floor::through("a1")),
        Err(RuleBreak::SignedPartInvalid {
            mid: "a3".into(),
            part: "r3".into()
        })
    );
}

#[test]
fn covered_signed_parts_never_open_a_gap() {
    let entries = signed_transcript([0, 0, 0]);
    // Without a floor, leaving out r3 between kept r1 and r5 invalidates r5.
    assert_eq!(invalid(&entries, &[drop_part(1, "a3", "r3")]), ["r5"]);
    // With r1 covered, r1 and r3 are one oldest run that is left out.
    assert!(invalid_under_floor(&entries, &[drop_part(1, "a3", "r3")], "a1").is_empty());
    // Dropping a covered signed part is not an edit that changes later parts.
    assert!(invalid_under_floor(&entries, &[drop_part(1, "a1", "r1")], "a3").is_empty());
}

#[test]
fn a_floor_naming_no_message_covers_nothing() {
    let entries = signed_transcript([0, 0, 0]);
    let head = vec![insert(1, InsertAt::Start, "m0")];
    assert_eq!(
        invalid_under_floor(&entries, &head, "another-branch"),
        invalid(&entries, &head)
    );
}

#[test]
fn plan_view_accepts_a_view_whose_edits_precede_only_covered_signed_parts() {
    let entries = strip_entries();
    // No anchor and no strip policy: without a floor, `plan_view` refuses the
    // view, because its start insert precedes the signed part.
    let view = strip_view(None, None);
    assert!(matches!(
        crate::render::plan_view(&entries, &view, &bound(), Floor::none()),
        Err(ViewRefusal::Rule(RuleBreak::SignedPartInvalid { .. }))
    ));
    let planned =
        crate::render::plan_view(&entries, &view, &bound(), Floor::through("reply")).unwrap();
    let crate::render::PlanItem::Entry { parts, .. } = planned.items.last().unwrap() else {
        panic!("the reply must remain");
    };
    // Checks only: the covered part stays in the plan for the runner's strip.
    assert_eq!(
        parts
            .iter()
            .map(|part| part.id.as_str())
            .collect::<Vec<_>>(),
        ["thinking", "call"]
    );
    assert_eq!(
        crate::render::plan_view(&entries, &view, &bound(), Floor::through("reply")),
        crate::render::plan(&entries, &view.edits),
        "a covered part draws no temporary drop"
    );
}

/// The same transcript in compact form, as a notice carries it.
fn compact(entries: &[Entry]) -> Vec<Entry> {
    entries
        .iter()
        .map(|entry| {
            let mut compact = entry.clone();
            compact.parts = entry.parts.iter().map(Part::compact).collect();
            compact
        })
        .collect()
}

#[test]
fn edits_computed_from_the_compact_form_apply_to_the_full_one_and_back() {
    let full = transcript();
    let short = compact(&full);
    // Computed by a provider that only holds the compact form.
    let edits = vec![
        replace(1, &short, "a1", "k1", "output", json!("[dropped §3§]")),
        replace(1, &short, "u1", "t1", "text", json!("fix it")),
        drop_part(1, "a1", "r1"),
    ];
    let on_full = apply_edits(&full, &edits).unwrap();
    let on_short = apply_edits(&short, &edits).unwrap();
    assert_eq!(on_full.len(), on_short.len());
    for (left, right) in on_full.iter().zip(&on_short) {
        let ids = |message: &crate::render::RenderedMessage| {
            message
                .parts
                .iter()
                .map(|part| (part.id.clone(), part.tag))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(left), ids(right));
    }
    let PartBody::Tool(tool) = &on_short[1].parts[1].body else {
        panic!()
    };
    assert_eq!(tool.output.as_deref(), Some("[dropped §3§]"));
    assert_eq!(tool.output_bytes, Some("[dropped §3§]".len() as u64));
    assert_eq!(
        invalid_signed_parts(&full, &edits, &bound(), Floor::none()),
        invalid_signed_parts(&short, &edits, &bound(), Floor::none())
    );
}

#[test]
fn an_inserted_message_carrying_a_tag_prefix_is_refused() {
    let entries = transcript();
    let mut tagged = insert(1, InsertAt::Start, "head");
    if let EditOp::Insert { message, .. } = &mut tagged.op {
        message.texts = vec!["summary that quotes \u{a7}3\u{a7} by mistake".into()];
    }
    assert_eq!(
        apply_edits(&entries, &[tagged]),
        Err(ViewRefusal::TaggedInsert { id: "head".into() })
    );
    assert!(apply_edits(&entries, &[insert(1, InsertAt::Start, "head")]).is_ok());
}

#[test]
fn a_replace_may_show_its_own_number_and_numbers_the_original_had_but_no_other() {
    let mut entries = transcript();
    entries[0].parts[0] = text("t1", 1, "please drop \u{a7}7\u{a7} and fix");
    let own = replace(
        1,
        &entries,
        "u1",
        "t1",
        "text",
        json!("[dropped \u{a7}1\u{a7}]"),
    );
    assert!(apply_edits(&entries, &[own]).is_ok());
    let kept = replace(
        1,
        &entries,
        "u1",
        "t1",
        "text",
        json!("drop \u{a7}7\u{a7}, fix"),
    );
    assert!(apply_edits(&entries, &[kept]).is_ok());
    let foreign = replace(1, &entries, "u1", "t1", "text", json!("see \u{a7}9\u{a7}"));
    assert_eq!(
        apply_edits(&entries, &[foreign]),
        Err(ViewRefusal::ForeignTag {
            mid: "u1".into(),
            part: "t1".into(),
            field: "text".into(),
            tag: 9,
        })
    );
}

/// Values a later contract version may add are kept and handled on the safe
/// side (W1.5): an unknown edit, anchor or inserted role refuses the view by
/// name; an unknown part kind takes no number and may only be dropped; an
/// unknown tool status counts as running; an output in an unknown format is
/// never rewritten.
#[test]
fn unknown_values_are_handled_on_the_safe_side() {
    let entries = transcript();
    let unknown_op = Edit::new(
        1,
        EditOp::Unknown {
            name: "fold".into(),
            fields: serde_json::Map::new(),
        },
    );
    assert_eq!(
        apply_edits(&entries, &[drop_part(1, "a1", "k1"), unknown_op]),
        Err(ViewRefusal::UnsupportedEdit { index: 1 })
    );
    let unknown_anchor = Edit::new(
        1,
        EditOp::Insert {
            at: InsertAt::Unknown(json!("end")),
            message: InsertedMessage::new("h", Role::User, vec!["x".into()]),
        },
    );
    assert_eq!(
        apply_edits(&entries, &[unknown_anchor]),
        Err(ViewRefusal::UnsupportedEdit { index: 0 })
    );
    let unknown_role = Edit::new(
        1,
        EditOp::Insert {
            at: InsertAt::Start,
            message: InsertedMessage::new("h", Role::Unknown("developer".into()), vec!["x".into()]),
        },
    );
    assert_eq!(
        apply_edits(&entries, &[unknown_role]),
        Err(ViewRefusal::UnsupportedEdit { index: 0 })
    );

    let mut entries = transcript();
    entries[1].parts.push(mk(
        "x1",
        None,
        Seal::None,
        PartBody::Unknown {
            name: "hologram".into(),
            fields: serde_json::Map::new(),
        },
    ));
    let mut queued = tool_body("k1", ToolStatus::Unknown("queued".into()));
    queued.output = Some("done".into());
    queued.output_bytes = Some(4);
    entries[1].parts[2] = mk("k1", Some(3), Seal::None, PartBody::Tool(queued));
    let mut xml = tool_body("k3", ToolStatus::Completed);
    xml.output_format = OutputFormat::Unknown("xml".into());
    entries[2]
        .parts
        .push(mk("k3", Some(5), Seal::None, PartBody::Tool(xml)));

    let mut numbered = entries.clone();
    crate::tags::assign_tags(0, &mut numbered);
    assert_eq!(numbered[1].parts[3].tag, None);
    assert!(apply_edits(&entries, &[drop_part(1, "a1", "x1")]).is_ok());
    assert!(matches!(
        apply_edits(
            &entries,
            &[replace(1, &entries, "a1", "x1", "text", json!("y"))]
        ),
        Err(ViewRefusal::BadField { .. })
    ));
    assert!(matches!(
        apply_edits(&entries, &[drop_part(1, "a1", "k1")]),
        Err(ViewRefusal::RunningTool { .. })
    ));
    assert!(matches!(
        apply_edits(
            &entries,
            &[replace(1, &entries, "u2", "k3", "output", json!("short"))]
        ),
        Err(ViewRefusal::BadField { .. })
    ));
    assert!(apply_edits(&entries, &[drop_part(1, "u2", "k3")]).is_ok());
}

#[test]
fn sealed_and_provider_run_parts_can_only_be_dropped() {
    let mut entries = transcript();
    entries[0].parts[0] = mk(
        "t1",
        Some(1),
        Seal::Signed,
        PartBody::Text {
            text: "signed text".into(),
        },
    );
    let mut hosted = tool_body("k9", ToolStatus::Completed);
    hosted.provider_executed = true;
    entries[1].parts[2] = mk("k1", None, Seal::None, PartBody::Tool(hosted));

    for (mid, part, field) in [("u1", "t1", "text"), ("a1", "k1", "output")] {
        let edit = replace(1, &entries, mid, part, field, json!("short"));
        assert_eq!(
            apply_edits(&entries, &[edit]),
            Err(ViewRefusal::BadField {
                mid: mid.into(),
                part: part.into(),
                field: field.into(),
            })
        );
        assert!(apply_edits(&entries, &[drop_part(1, mid, part)]).is_ok());
    }
}

#[test]
fn a_replace_of_delivered_keeps_only_the_named_sends() {
    let mut entries = transcript();
    let mut body = tool_body("k1", ToolStatus::Completed);
    body.set_delivered(vec![
        Delivered::new("provider-a", "first").with_template("provider.reduce"),
        Delivered::new("provider-b", "second"),
    ]);
    entries[1].parts[2] = mk("k1", Some(3), Seal::None, PartBody::Tool(body));
    let keep_second = replace(1, &entries, "a1", "k1", "delivered", json!([1]));
    let rendered = apply_edits(&entries, &[keep_second]).unwrap();
    let PartBody::Tool(tool) = &rendered[1].parts[2].body else {
        panic!("the tool part stays a tool part");
    };
    let delivered = tool.delivered().unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].text.as_deref(), Some("second"));

    for bad in [json!([2]), json!([0, 0]), json!(["0"])] {
        let edit = replace(1, &entries, "a1", "k1", "delivered", bad);
        assert!(matches!(
            apply_edits(&entries, &[edit]),
            Err(ViewRefusal::BadField { .. })
        ));
    }
}

/// The rides a plan lists for one tool part, as (index, text).
fn planned_rides(plan: &crate::render::RenderPlan, part_id: &str) -> Vec<(u64, String)> {
    plan.items
        .iter()
        .filter_map(|item| match item {
            crate::render::PlanItem::Entry { parts, .. } => {
                parts.iter().find(|part| part.id == part_id)
            }
            _ => None,
        })
        .flat_map(|part| {
            part.rides.iter().map(|ride| {
                let text = ride.delivered.text.as_deref().unwrap_or_default();
                (ride.index, text.to_owned())
            })
        })
        .collect()
}

#[test]
fn the_plan_lists_a_results_rides_and_a_replaced_output_takes_them_with_it() {
    let mut entries = transcript();
    let mut body = tool_body("k1", ToolStatus::Completed);
    body.set_delivered(vec![
        Delivered::new("provider-a", "first"),
        Delivered::new("provider-b", "second"),
    ]);
    entries[1].parts[2] = mk("k1", Some(3), Seal::None, PartBody::Tool(body));
    let both = vec![(0, "first".to_owned()), (1, "second".to_owned())];

    // Untouched, the result keeps both rides, in delivery order.
    assert_eq!(
        planned_rides(&crate::render::plan(&entries, &[]).unwrap(), "k1"),
        both
    );

    // A placeholder takes them with it, in the plan and in what the model sees.
    let placeholder = replace(1, &entries, "a1", "k1", "output", json!("[dropped §3§]"));
    let edits = std::slice::from_ref(&placeholder);
    assert!(planned_rides(&crate::render::plan(&entries, edits).unwrap(), "k1").is_empty());
    let rendered = apply_edits(&entries, edits).unwrap();
    let PartBody::Tool(tool) = &rendered[1].parts[2].body else {
        panic!("the tool part stays a tool part");
    };
    assert!(tool.delivered().is_none());

    // Version 2 kept them after a replaced output, as runners rendered then.
    let at_two = crate::render::plan_at(2, &entries, edits).unwrap().unwrap();
    assert_eq!(planned_rides(&at_two, "k1"), both);
    assert!(crate::render::plan_at(1, &entries, edits).is_none());

    // A view that names `delivered` itself keeps its choice.
    let keep_second = replace(2, &entries, "a1", "k1", "delivered", json!([1]));
    let edits = [placeholder, keep_second];
    assert_eq!(
        planned_rides(&crate::render::plan(&entries, &edits).unwrap(), "k1"),
        vec![(1, "second".to_owned())]
    );
    let rendered = apply_edits(&entries, &edits).unwrap();
    let PartBody::Tool(tool) = &rendered[1].parts[2].body else {
        panic!("the tool part stays a tool part");
    };
    let delivered = tool.delivered().unwrap();
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].text.as_deref(), Some("second"));
}

#[test]
fn a_signed_text_part_is_bound_like_signed_reasoning() {
    // Gemini signs text and tool calls, not only reasoning. A kept signed
    // text part is invalidated by a later edit before it, like reasoning.
    let mut entries = transcript();
    entries[2].parts[0] = mk(
        "t3",
        Some(4),
        Seal::Signed,
        PartBody::Text {
            text: "go on".into(),
        },
    );
    let invalid = invalid_signed_parts(
        &entries,
        &[drop_part(1, "u1", "t1")],
        &bound(),
        Floor::none(),
    );
    assert!(invalid.contains(&("u2".to_owned(), "t3".to_owned())));
}

/// Edits inserting two user messages at the start, with ids `m0` and `m1` and
/// the given texts, the way a compaction provider places its history.
fn head(m0: &str, m1: &str) -> Vec<Edit> {
    [("m0", m0), ("m1", m1)]
        .into_iter()
        .map(|(id, text)| {
            Edit::new(
                1,
                EditOp::Insert {
                    at: InsertAt::Start,
                    message: InsertedMessage::new(id, Role::User, vec![text.into()]),
                },
            )
        })
        .collect()
}

#[test]
fn first_change_names_the_first_item_a_newer_plan_renders_differently() {
    let entries = transcript();
    let planned = |edits: Vec<Edit>| plan(&entries, &edits).unwrap();
    let current = planned(head("history", "recent"));

    assert_eq!(first_change(&current, &current), FirstChange::Unchanged);

    // Changing m0's text changes the request's first message.
    let fold = first_change(&current, &planned(head("history, folded", "recent")));
    assert_eq!(
        fold,
        FirstChange::Insert {
            item: 0,
            id: "m0".into()
        }
    );
    assert!(fold.from_first_message());

    // Changing m1's text keeps m0, the first message.
    let refresh = first_change(&current, &planned(head("history", "recent, newer")));
    assert_eq!(
        refresh,
        FirstChange::Insert {
            item: 1,
            id: "m1".into()
        }
    );
    assert!(!refresh.from_first_message());

    // Dropping part t2 from entry a1 keeps m0, m1 and u1, the entry before a1.
    let mut dropped = head("history", "recent");
    dropped.push(drop_part(1, "a1", "t2"));
    assert_eq!(
        first_change(&current, &planned(dropped)),
        FirstChange::Entry {
            item: 3,
            ordinal: 1,
            mid: "a1".into()
        }
    );

    // Going from no edits to the two inserts changes the first message: the
    // request now starts with m0.
    assert!(first_change(&planned(vec![]), &current).from_first_message());
}

#[test]
fn a_cleared_ride_renders_cleared_only_on_a_route_that_clears() {
    use cortexkit_role_compaction::part::RideForm;
    assert_eq!(
        cleared_ride(RideForm::TurnSystemCleared, true),
        RideRender::Cleared
    );
    // Never a persistent system message on a route that can't clear.
    assert_eq!(
        cleared_ride(RideForm::TurnSystemCleared, false),
        RideRender::Omit
    );
    for form in [RideForm::ToolResult, RideForm::TurnSystem] {
        for route_clears in [true, false] {
            assert_eq!(cleared_ride(form, route_clears), RideRender::Form(form));
        }
    }
}

#[test]
fn first_change_reports_where_a_shorter_plan_ends() {
    let entries = transcript();
    let edits = head("history", "recent");
    let full = plan(&entries, &edits).unwrap();
    let fewer = plan(&entries[..2], &edits).unwrap();
    assert_eq!(first_change(&fewer, &full), FirstChange::End { item: 4 });
    assert_eq!(first_change(&full, &fewer), FirstChange::End { item: 4 });
    assert_eq!(FirstChange::Unchanged.item(), None);
}

/// The JSON test vectors under `vectors/cases.json`, for runners that use the
/// library through WebAssembly or implement the rules themselves. Each case
/// is generated here and its output computed by [`crate::vectors::call`].
/// Set `UPDATE_VECTORS=1` to rewrite the file after a deliberate change; the
/// test otherwise fails when the file and the library disagree.
mod vector_file {
    use super::*;
    use crate::vectors::{call, Vector, FUNCTIONS};
    use cortexkit_role_compaction::read::NoticeOrigin;
    use serde_json::{json, Value};

    /// A transcript as a runner presents it: ordinals count from 1.
    /// u1 | a1: r1 (signed), t2, k1 (edit with an image) | u2 | a2: r2 (signed), t5
    fn transcript(generated: [u64; 2]) -> Vec<Entry> {
        vec![
            entry(1, "u1", Role::User, vec![text("t1", 1, "please fix")], None),
            entry(
                2,
                "a1",
                Role::Assistant,
                vec![
                    signed("r1"),
                    text("t2", 2, "looking"),
                    tool("k1", 3, ToolStatus::Completed),
                ],
                Some(generated[0]),
            ),
            entry(3, "u2", Role::User, vec![text("t4", 4, "go on")], None),
            entry(
                4,
                "a2",
                Role::Assistant,
                vec![signed("r2"), text("t5", 5, "done")],
                Some(generated[1]),
            ),
        ]
    }

    fn running() -> Vec<Entry> {
        vec![
            entry(1, "u1", Role::User, vec![text("t1", 1, "run it")], None),
            entry(
                2,
                "a1",
                Role::Assistant,
                vec![tool("k1", 2, ToolStatus::Running)],
                None,
            ),
        ]
    }

    fn only_image() -> Vec<Entry> {
        let mut body = tool_body("k1", ToolStatus::Completed);
        body.output = Some("".into());
        body.output_bytes = Some(0);
        vec![
            entry(1, "u1", Role::User, vec![text("t1", 1, "screenshot")], None),
            entry(
                2,
                "a1",
                Role::Assistant,
                vec![mk("k1", Some(2), Seal::None, PartBody::Tool(body))],
                None,
            ),
        ]
    }

    fn head(since: u64) -> Edit {
        Edit::new(
            since,
            EditOp::Insert {
                at: InsertAt::Start,
                message: InsertedMessage::new(
                    "m0",
                    Role::User,
                    vec!["<session-history>summary</session-history>".into()],
                ),
            },
        )
    }

    /// A start insert carrying one image: a one-pixel PNG unless `data` says
    /// otherwise.
    fn image_head(role: Role, media_type: &str, data: &str) -> Edit {
        Edit::new(
            1,
            EditOp::Insert {
                at: InsertAt::Start,
                message: InsertedMessage::new("m0", role, vec!["<memory-mural/>".into()])
                    .with_images(vec![InsertedImage::new(media_type, data)]),
            },
        )
    }

    const PIXEL: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    fn cut(since: u64, through: &str) -> Edit {
        Edit::new(
            since,
            EditOp::Cut {
                through: through.into(),
            },
        )
    }

    fn free() -> ProviderRules {
        ProviderRules::new(SignedParts::None, EditCost::Suffix)
    }

    fn cases() -> Vec<(&'static str, &'static str, Value)> {
        let t = transcript([0, 0]);
        let plan = |edits: Vec<Edit>| json!({"entries": t, "edits": edits});
        // The transcript up to its second user message, which then ends it.
        let user_last: Vec<Entry> = transcript([0, 0]).into_iter().take(3).collect();
        let calls_then_empty = calls_then_empty();
        let one_system = calls_then_systems(1);
        let two_systems = calls_then_systems(2);
        let mut user_notice = calls_then_empty[..3].to_vec();
        user_notice.push(
            entry(
                4,
                "n1",
                Role::User,
                vec![text("nt1", 5, "please continue")],
                None,
            )
            .with_notice(NoticeOrigin::new("example-notifier")),
        );
        let placeholder = replace(1, &t, "u2", "t4", "text", json!("[dropped §4§]"));
        let own_output = replace(1, &t, "a1", "k1", "output", json!("[dropped §3§]"));
        let marker = replace(
            1,
            &t,
            "a1",
            "k1",
            "input.old_string",
            json!("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx…[truncated]"),
        );
        let foreign = replace(1, &t, "u2", "t4", "text", json!("see §9§"));
        let stale = {
            let mut edit = replace(1, &t, "u2", "t4", "text", json!("[dropped §4§]"));
            if let EditOp::Replace { expect, .. } = &mut edit.op {
                *expect = "0".repeat(64).into();
            }
            edit
        };
        let tagged_head = Edit::new(
            1,
            EditOp::Insert {
                at: InsertAt::Start,
                message: InsertedMessage::new("m0", Role::User, vec!["§2§ summary".into()]),
            },
        );
        let view = View::seal(1, vec![drop_part(1, "a1", "k1")]);
        let mut bad_checksum = view.clone();
        bad_checksum.checksum = "0".repeat(64).into();
        let previous = View::seal(1, vec![drop_part(1, "a1", "k1")]);
        let carried = View::seal(2, vec![drop_part(1, "a1", "k1"), drop_part(2, "u2", "t4")]);
        let backdated = View::seal(2, vec![drop_part(1, "a1", "k1"), drop_part(1, "u2", "t4")]);
        let image = only_image();
        let mut unnumbered = transcript([0, 0]);
        for entry in &mut unnumbered {
            for part in &mut entry.parts {
                part.tag = None;
            }
        }
        let pasted = vec![
            entry(
                1,
                "u1",
                Role::User,
                vec![mk(
                    "f1",
                    None,
                    Seal::None,
                    PartBody::File(FileMedia::inline("image/png", Some(4096))),
                )],
                None,
            ),
            entry(
                2,
                "a1",
                Role::Assistant,
                vec![text("t1", 1, "I see it")],
                None,
            ),
        ];
        let stripped_image = replace(1, &pasted, "u1", "f1", "text", json!("[image stripped]"));
        let mut ridden = transcript([0, 0]);
        let mut ridden_body = tool_body("k1", ToolStatus::Completed);
        ridden_body.set_delivered(vec![Delivered::new(
            "compaction-provider",
            "<system-reminder>\nspent\n</system-reminder>",
        )]);
        ridden[1].parts[2] = mk("k1", Some(3), Seal::None, PartBody::Tool(ridden_body));
        let ridden_output = replace(1, &ridden, "a1", "k1", "output", json!("[dropped §3§]"));
        let keep_ride = replace(2, &ridden, "a1", "k1", "delivered", json!([0]));
        vec![
            (
                "the version of what this library renders",
                "plan_version",
                json!({}),
            ),
            ("an empty view keeps everything", "plan", plan(vec![])),
            (
                "an edit op the runner doesn't know refuses the view by name",
                "plan",
                plan(vec![
                    drop_part(1, "a1", "k1"),
                    Edit::new(
                        1,
                        EditOp::Unknown {
                            name: "fold".into(),
                            fields: serde_json::Map::new(),
                        },
                    ),
                ]),
            ),
            (
                "an insert anchor the runner doesn't know refuses the view by name",
                "plan",
                plan(vec![Edit::new(
                    1,
                    EditOp::Insert {
                        at: InsertAt::Unknown(json!("end")),
                        message: InsertedMessage::new("h", Role::User, vec!["x".into()]),
                    },
                )]),
            ),
            (
                "text takes an answered image's place",
                "plan",
                json!({"entries": pasted, "edits": [stripped_image]}),
            ),
            (
                "a dropped part is left out",
                "plan",
                plan(vec![drop_part(1, "a1", "k1")]),
            ),
            (
                "a dropped message is left out",
                "plan",
                plan(vec![drop_message(1, "u2")]),
            ),
            (
                "a message whose every part is dropped is left out",
                "plan",
                plan(vec![drop_part(1, "u2", "t4")]),
            ),
            (
                "a text placeholder replaces the text and is final",
                "plan",
                plan(vec![placeholder]),
            ),
            (
                "a tool result may show its own number",
                "plan",
                plan(vec![own_output]),
            ),
            (
                "an edit marker shortens one argument",
                "plan",
                plan(vec![marker]),
            ),
            (
                "a history head at the start with a cut",
                "plan",
                plan(vec![head(1), cut(1, "a1")]),
            ),
            (
                "an edit naming a message the transcript lacks is refused",
                "plan",
                plan(vec![drop_message(1, "gone")]),
            ),
            (
                "a replace computed from another part is refused",
                "plan",
                plan(vec![stale]),
            ),
            (
                "an inserted message may not carry a tag prefix",
                "plan",
                plan(vec![tagged_head]),
            ),
            (
                "a replace may not show another part's number",
                "plan",
                plan(vec![foreign]),
            ),
            (
                "two edits on one part that cannot both hold are refused",
                "plan",
                plan(vec![
                    drop_part(1, "u2", "t4"),
                    replace(1, &t, "u2", "t4", "text", json!("x")),
                ]),
            ),
            (
                "a running tool cannot be dropped",
                "plan",
                json!({"entries": running(), "edits": [drop_part(1, "a1", "k1")]}),
            ),
            (
                "a result with only an image renders a label",
                "plan",
                json!({"entries": image, "edits": []}),
            ),
            (
                "a sealed view applies",
                "apply",
                json!({"entries": t, "view": view}),
            ),
            (
                "a view whose checksum does not match is refused",
                "apply",
                json!({"entries": t, "view": bad_checksum}),
            ),
            (
                "edits for messages a revert removed are skipped",
                "edits_for_transcript",
                json!({"entries": &t[..2], "edits": [drop_part(1, "a1", "k1"), drop_part(1, "u2", "t4")]}),
            ),
            (
                "a new view repeats the previous one's edits",
                "check_carried",
                json!({"previous": previous, "next": carried}),
            ),
            (
                "a backdated edit is refused",
                "check_carried",
                json!({"previous": previous, "next": backdated}),
            ),
            (
                "an edit before a signed part made after it invalidates it",
                "check_rules",
                json!({"entries": t, "edits": [drop_part(1, "a1", "k1")], "rules": bound()}),
            ),
            (
                "an edit the signed part was generated under is fine",
                "check_rules",
                json!({"entries": transcript([0, 1]), "edits": [drop_part(1, "a1", "k1")], "rules": bound()}),
            ),
            (
                "the oldest signed part may be left out",
                "check_rules",
                json!({"entries": t, "edits": [drop_part(1, "a1", "r1")], "rules": bound()}),
            ),
            (
                "a history head invalidates every older signed part",
                "check_rules",
                json!({"entries": t, "edits": [head(1)], "rules": bound()}),
            ),
            (
                "without bound signatures nothing is invalid",
                "check_rules",
                json!({"entries": t, "edits": [drop_part(1, "a1", "k1")], "rules": free()}),
            ),
            (
                "a signed floor at the newest message covers every signed part",
                "check_rules",
                json!({"entries": t, "edits": [head(1)], "rules": bound(), "signed_floor": "a2"}),
            ),
            (
                "a history head still invalidates a signed part after the floor",
                "check_rules",
                json!({"entries": t, "edits": [head(1)], "rules": bound(), "signed_floor": "a1"}),
            ),
            (
                "an edit before the floor invalidates a signed part after it",
                "check_rules",
                json!({"entries": t, "edits": [drop_part(1, "a1", "k1")], "rules": bound(), "signed_floor": "a1"}),
            ),
            (
                "a signed floor naming no message covers nothing",
                "check_rules",
                json!({"entries": t, "edits": [head(1)], "rules": bound(), "signed_floor": "another-branch"}),
            ),
            (
                "current Claude families are bound",
                "rules_defaults",
                json!({"model": "claude-opus-5-5"}),
            ),
            (
                "other models are not",
                "rules_defaults",
                json!({"model": "gpt-6.1"}),
            ),
            (
                "numbers count from the checkpoint",
                "assign_tags",
                json!({"checkpoint": 41, "entries": unnumbered}),
            ),
            (
                "numbers are found anywhere in a text",
                "tag_numbers_in",
                json!({"text": "[dropped §12§] then §3§, but not §§ or §7"}),
            ),
            (
                "a reply's own prefix is split off",
                "clean_reply",
                json!({"text": "§12§ Done."}),
            ),
            (
                "numbers inside a reply are removed",
                "clean_reply",
                json!({"text": "§12§ §13§ Dropped §4§ and §5\">§6§ as asked."}),
            ),
            (
                "a section reference keeps its number but not its sign",
                "clean_reply",
                json!({"text": "See §5.1 of the spec."}),
            ),
            (
                "a cleaned reply is trimmed",
                "clean_reply",
                json!({"text": "  §12§ Done.\n\n"}),
            ),
            (
                "a first prefix too large for a number names no tag, and a later one never stands in",
                "clean_reply",
                json!({"text": "§18446744073709551616§ §7§ Done."}),
            ),
            (
                "a user insert's PNG renders after its texts",
                "plan",
                plan(vec![image_head(Role::User, "image/png", PIXEL)]),
            ),
            (
                "an image on an assistant insert is refused as unsupported_edit naming the edit index",
                "plan",
                plan(vec![image_head(Role::Assistant, "image/png", PIXEL)]),
            ),
            (
                "an insert declaring a media type other than image/png is refused as unsupported_edit naming the edit index",
                "plan",
                plan(vec![image_head(Role::User, "image/jpeg", PIXEL)]),
            ),
            (
                "image data that isn't standard padded base64 is refused as unsupported_edit naming the edit index",
                "plan",
                plan(vec![image_head(Role::User, "image/png", "iVBORw0K-_8=")]),
            ),
            (
                "an inserted image renders as an inline file part with its size",
                "apply",
                json!({"entries": t, "view": View::seal(1, vec![image_head(Role::User, "image/png", PIXEL)])}),
            ),
            (
                "an entry's checksum",
                "entry_digest",
                json!({"role": "user", "parts": [text("t1", 1, "please fix")]}),
            ),
            (
                "a part's checksum covers its seal and body",
                "body_digest",
                json!({"seal": "signed", "kind": "reasoning", "text": "thinking", "text_bytes": 8}),
            ),
            (
                "a view's checksum",
                "seal_view",
                json!({"version": 3, "edits": [drop_part(2, "a1", "k1")]}),
            ),
            // Cases added after a version was pinned go here, at the end, and
            // are pinned by a longer prefix in `PLAN_OUTPUTS`. These
            // reply-cleaning inputs are the ones an earlier runner-side
            // cleaner was compared against; they agreed on every one.
            (
                "reply cleaning edge case: two leading tags",
                "clean_reply",
                json!({"text": "§1§ §2§ Done."}),
            ),
            (
                "reply cleaning edge case: a tag after the text",
                "clean_reply",
                json!({"text": "Done §3§."}),
            ),
            (
                "reply cleaning edge case: a section reference",
                "clean_reply",
                json!({"text": "§5.1 applies"}),
            ),
            (
                "reply cleaning edge case: two section references",
                "clean_reply",
                json!({"text": "§12.3 and §4"}),
            ),
            (
                "reply cleaning edge case: a broken tag ending",
                "clean_reply",
                json!({"text": "§9\"> broken"}),
            ),
            (
                "reply cleaning edge case: a broken tag ending before a tag",
                "clean_reply",
                json!({"text": "§9\">§10§ text"}),
            ),
            (
                "reply cleaning edge case: a section sign before a colon",
                "clean_reply",
                json!({"text": "§7: note"}),
            ),
            (
                "reply cleaning edge case: a section sign before letters",
                "clean_reply",
                json!({"text": "§7a word"}),
            ),
            (
                "reply cleaning edge case: a lone trailing section sign",
                "clean_reply",
                json!({"text": "trailing §"}),
            ),
            (
                "reply cleaning edge case: a doubled section sign",
                "clean_reply",
                json!({"text": "§§ double"}),
            ),
            (
                "reply cleaning edge case: a reply that is only a tag",
                "clean_reply",
                json!({"text": "§1§"}),
            ),
            (
                "reply cleaning edge case: a padded tag",
                "clean_reply",
                json!({"text": "  §2§  padded  "}),
            ),
            (
                "reply cleaning edge case: a section sign before a count",
                "clean_reply",
                json!({"text": "§99 files"}),
            ),
            (
                "reply cleaning edge case: no section sign",
                "clean_reply",
                json!({"text": "2024 roadmap"}),
            ),
            (
                "reply cleaning edge case: tags on separate lines",
                "clean_reply",
                json!({"text": "§1§\n\n§2§ multi"}),
            ),
            (
                "reply cleaning edge case: adjacent tags inside text",
                "clean_reply",
                json!({"text": "a §3§§4§ b"}),
            ),
            (
                "reply cleaning edge case: letters between section signs",
                "clean_reply",
                json!({"text": "§x§ not a tag"}),
            ),
            (
                "reply cleaning edge case: tag zero",
                "clean_reply",
                json!({"text": "§0§ zero"}),
            ),
            (
                "reply cleaning edge case: an oversized tag alone",
                "clean_reply",
                json!({"text": "§18446744073709551616§ big"}),
            ),
            (
                "reply cleaning edge case: non-breaking spaces around a tag",
                "clean_reply",
                json!({"text": "\u{a0}§5§\u{a0}nbsp"}),
            ),
            (
                "reply cleaning edge case: a section sign before a parenthesis",
                "clean_reply",
                json!({"text": "§6) closer"}),
            ),
            (
                "reply cleaning edge case: a section sign before a bracket",
                "clean_reply",
                json!({"text": "§6] closer"}),
            ),
            (
                "reply cleaning edge case: a section sign before a dash",
                "clean_reply",
                json!({"text": "§6- dash"}),
            ),
            (
                "reply cleaning edge case: a section sign before an underscore",
                "clean_reply",
                json!({"text": "§6_ under"}),
            ),
            // Version 3: the plan lists a result's rides, and a replaced
            // output takes them with it.
            (
                "the plan lists the sends that ride a tool result",
                "plan",
                json!({"entries": ridden.clone(), "edits": []}),
            ),
            (
                "replacing a result drops the sends that rode it",
                "plan",
                json!({"entries": ridden.clone(), "edits": [ridden_output.clone()]}),
            ),
            (
                "a replaced result renders without the sends that rode it",
                "apply",
                json!({"entries": ridden.clone(), "view": View::seal(1, vec![ridden_output.clone()])}),
            ),
            (
                "version 2 keeps the rides after a replaced output",
                "plan_at",
                json!({"version": 2, "entries": ridden.clone(), "edits": [ridden_output.clone()]}),
            ),
            (
                "a view that names delivered keeps the rides it lists",
                "plan",
                json!({"entries": ridden.clone(), "edits": [ridden_output, keep_ride]}),
            ),
            // first_change: `before` is the plan of the applied view and
            // `after` the plan of a newer one, both over the same transcript.
            (
                "changing m0's text changes the request's first message",
                "first_change",
                json!({"before": head_plan("history", "recent"), "after": head_plan("history, folded", "recent")}),
            ),
            (
                "changing m1's text keeps m0",
                "first_change",
                json!({"before": head_plan("history", "recent"), "after": head_plan("history", "recent, newer")}),
            ),
            (
                "the same plan renders unchanged",
                "first_change",
                json!({"before": head_plan("history", "recent"), "after": head_plan("history", "recent")}),
            ),
            // The final turn (the compaction contract's rule PR6): the request ends on the newest
            // entry that has parts, and a view never removes what ends it.
            (
                "a view can't drop every part of the entry the request ends on",
                "plan",
                plan(vec![drop_part(1, "a2", "r2"), drop_part(1, "a2", "t5")]),
            ),
            (
                "a cut through the entry the request ends on is refused",
                "plan",
                plan(vec![cut(1, "a2")]),
            ),
            (
                "a view can't drop the newest user message",
                "plan",
                json!({"entries": user_last.clone(), "edits": [drop_part(1, "u2", "t4")]}),
            ),
            (
                "the newest user message's text may become its placeholder",
                "plan",
                json!({"entries": user_last.clone(), "edits": [replace(1, &user_last, "u2", "t4", "text", json!("[dropped §4§]"))]}),
            ),
            (
                "empty entries after the last call don't move the end: dropping that call is refused",
                "plan",
                json!({"entries": calls_then_empty.clone(), "edits": [drop_part(1, "a1", "k2")]}),
            ),
            (
                "an earlier call of that entry may go whole",
                "plan",
                json!({"entries": calls_then_empty.clone(), "edits": [drop_part(1, "a1", "k1")]}),
            ),
            (
                "the last call may lose its output and stay a call",
                "plan",
                json!({"entries": calls_then_empty.clone(), "edits": [replace(1, &calls_then_empty, "a1", "k2", "output", json!("[dropped §4§]"))]}),
            ),
            (
                "one trailing system entry does not let the final call go",
                "plan",
                json!({"entries": one_system.clone(), "edits": [drop_part(1, "a1", "k1"), drop_part(1, "a1", "k2")]}),
            ),
            (
                "one trailing system entry still lets an earlier call go",
                "plan",
                json!({"entries": one_system.clone(), "edits": [drop_part(1, "a1", "k1")]}),
            ),
            (
                "two trailing system entries do not let the call entry go whole",
                "plan",
                json!({"entries": two_systems.clone(), "edits": [drop_message(1, "u1"), drop_message(1, "a1")]}),
            ),
            (
                "two trailing system entries still let an earlier call go",
                "plan",
                json!({"entries": two_systems.clone(), "edits": [drop_part(1, "a1", "k1")]}),
            ),
            (
                "a trailing user notice lets the call entry go whole",
                "plan",
                json!({"entries": user_notice, "edits": [drop_message(1, "a1")]}),
            ),
            (
                "the newest system entry remains protected too",
                "plan",
                json!({"entries": one_system, "edits": [drop_message(1, "s1")]}),
            ),
            (
                "an earlier trailing system entry is not additionally protected",
                "plan",
                json!({"entries": two_systems.clone(), "edits": [drop_message(1, "s1")]}),
            ),
            (
                "a cut cannot reach the last non-system entry before trailing systems",
                "plan",
                json!({"entries": two_systems.clone(), "edits": [cut(1, "a1")]}),
            ),
            (
                "the protected call before trailing systems may lose its output",
                "plan",
                json!({"entries": two_systems.clone(), "edits": [replace(1, &two_systems, "a1", "k2", "output", json!("[dropped §4§]"))]}),
            ),
            (
                "an earlier plan version also protects the call before trailing systems",
                "plan_at",
                json!({"version": 2, "entries": two_systems, "edits": [drop_message(1, "a1")]}),
            ),
            (
                "strip an invalidated signed part newer than the anchor and keep the newest assistant tool",
                "plan_view",
                json!({"entries": strip_entries(), "view": strip_view(Some("anchor"), Some("strip")), "rules": bound()}),
            ),
            (
                "a missing built_through refuses an invalidated signature",
                "plan_view",
                json!({"entries": strip_entries(), "view": strip_view(None, Some("strip")), "rules": bound()}),
            ),
            (
                "an anchor off the active path refuses an invalidated signature",
                "plan_view",
                json!({"entries": strip_entries(), "view": strip_view(Some("removed-branch"), Some("strip")), "rules": bound()}),
            ),
            (
                "a signed part at the anchor is never stripped",
                "plan_view",
                json!({"entries": strip_entries(), "view": strip_view(Some("reply"), Some("strip")), "rules": bound()}),
            ),
            (
                "an unknown new_signed policy refuses an invalidated signature",
                "plan_view",
                json!({"entries": strip_entries(), "view": strip_view(Some("anchor"), Some("future-policy")), "rules": bound()}),
            ),
            (
                "an absent new_signed policy refuses an invalidated signature",
                "plan_view",
                json!({"entries": strip_entries(), "view": strip_view(Some("anchor"), None), "rules": bound()}),
            ),

            (
                "versioned view-aware planning strips newer signatures",
                "plan_view_at",
                json!({"version": 3, "entries": strip_entries(), "view": strip_view(Some("anchor"), Some("strip")), "rules": bound()}),
            ),
            (
                "view-aware application strips only the newer signed part",
                "apply_with_rules",
                json!({"entries": strip_entries(), "view": strip_view(Some("anchor"), Some("strip")), "rules": bound()}),
            ),
            (
                "view checksum includes both signature policy members",
                "seal_view",
                json!({"version": 1, "edits": [], "built_through": "anchor", "new_signed": "strip"}),
            ),
            (
                "view checksum includes an anchor without a policy",
                "seal_view",
                json!({"version": 1, "edits": [], "built_through": "anchor"}),
            ),
            (
                "view checksum includes an unknown policy without an anchor",
                "seal_view",
                json!({"version": 1, "edits": [], "new_signed": "future-policy"}),
            ),
            (
                "view checksum without optional signature policy members is unchanged",
                "seal_view",
                json!({"version": 1, "edits": []}),
            ),
            // cleared_ride: a cleared ride renders cleared only on a route
            // that clears, and is never a persistent system message.
            (
                "a cleared ride renders cleared on a route that clears",
                "cleared_ride",
                json!({"form": "turn_system_cleared", "route_clears": true}),
            ),
            (
                "a cleared ride is omitted on a route that can't clear",
                "cleared_ride",
                json!({"form": "turn_system_cleared", "route_clears": false}),
            ),
            (
                "a turn_system ride keeps its form on any route",
                "cleared_ride",
                json!({"form": "turn_system", "route_clears": false}),
            ),
            // A signed floor decides validity only: the covered part stays in
            // the plan and the rendered messages for the runner's own strip.
            (
                "a signed floor accepts a view that edits before covered signed parts and keeps them in the plan",
                "plan_view",
                json!({"entries": strip_entries(), "view": strip_view(None, None), "rules": bound(), "signed_floor": "reply"}),
            ),
            (
                "apply_with_rules under a signed floor keeps the covered part for the runner's strip",
                "apply_with_rules",
                json!({"entries": strip_entries(), "view": strip_view(None, None), "rules": bound(), "signed_floor": "reply"}),
            ),
            // Version 4: a removed mid-text tag no longer leaves a doubled
            // space, or a space before closing punctuation. Everything else
            // a removed tag sits next to stays as the model wrote it.
            (
                "reply cleaning, version 4: a removed tag between two spaces keeps one space",
                "clean_reply",
                json!({"text": "the file §4§ holds it"}),
            ),
            (
                "reply cleaning, version 4: a removed tag before a full stop takes its space with it",
                "clean_reply",
                json!({"text": "see §3§."}),
            ),
            (
                "reply cleaning, version 4: a removed tag before a comma takes its space with it",
                "clean_reply",
                json!({"text": "first §3§, then"}),
            ),
            (
                "reply cleaning, version 4: a removed tag before a semicolon takes its space with it",
                "clean_reply",
                json!({"text": "one §3§; two"}),
            ),
            (
                "reply cleaning, version 4: a removed tag before a colon takes its space with it",
                "clean_reply",
                json!({"text": "as follows §3§: this"}),
            ),
            (
                "reply cleaning, version 4: a removed tag before an exclamation mark takes its space with it",
                "clean_reply",
                json!({"text": "done §3§!"}),
            ),
            (
                "reply cleaning, version 4: a removed tag before a question mark takes its space with it",
                "clean_reply",
                json!({"text": "right §3§?"}),
            ),
            (
                "reply cleaning, version 4: a removed tag before a closing parenthesis takes its space with it",
                "clean_reply",
                json!({"text": "(see §3§) next"}),
            ),
            (
                "reply cleaning, version 4: a removed tag before a closing bracket takes its space with it",
                "clean_reply",
                json!({"text": "[see §3§] next"}),
            ),
            (
                "reply cleaning, version 4: spaced tags before punctuation go with their spaces",
                "clean_reply",
                json!({"text": "x §3§ §4§."}),
            ),
            (
                "reply cleaning, version 4: spaced tags between words leave one space",
                "clean_reply",
                json!({"text": "a §3§ §4§ b"}),
            ),
            (
                "reply cleaning, version 4: a model-written double space stays",
                "clean_reply",
                json!({"text": "model  wrote  doubles §3§ here"}),
            ),
            (
                "reply cleaning, version 4: a model-written double space before a tag stays",
                "clean_reply",
                json!({"text": "word  §3§ next"}),
            ),
            (
                "reply cleaning, version 4: a tag before a newline leaves its space",
                "clean_reply",
                json!({"text": "line §3§\nnext"}),
            ),
            (
                "reply cleaning, version 4: a tag after a newline leaves its space",
                "clean_reply",
                json!({"text": "line\n§3§ next"}),
            ),
            (
                "reply cleaning, version 4: a tag at the very start is split off as before",
                "clean_reply",
                json!({"text": "§3§ start"}),
            ),
            (
                "reply cleaning, version 4: a tag at the very end is trimmed as before",
                "clean_reply",
                json!({"text": "end §3§"}),
            ),
            (
                "reply cleaning, version 4: two adjacent tags between words leave both spaces",
                "clean_reply",
                json!({"text": "a §3§§4§ b"}),
            ),
            (
                "reply cleaning, version 4: two adjacent tags before punctuation leave the space",
                "clean_reply",
                json!({"text": "see §3§§4§."}),
            ),
            (
                "reply cleaning, version 4: a broken tag between words leaves both spaces",
                "clean_reply",
                json!({"text": "mid §11865\">§ text"}),
            ),
            (
                "reply cleaning, version 4: a dangling tag before punctuation leaves its space",
                "clean_reply",
                json!({"text": "see §12$."}),
            ),
            (
                "reply cleaning, version 4: tabs around a tag stay",
                "clean_reply",
                json!({"text": "a\t§3§\tb"}),
            ),
            (
                "reply cleaning, version 4: non-breaking spaces around a tag stay",
                "clean_reply",
                json!({"text": "a\u{a0}§3§\u{a0}b"}),
            ),
            // The same inputs as plan version 3 cleaned them, through the
            // versioned entry point a runner uses for a run recorded under 3,
            // and a few through it at 4 and at 2.
            (
                "plan version 3: a tag inside a reply keeps both spaces",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§12§ §13§ Dropped §4§ and §5\">§6§ as asked."}),
            ),
            (
                "plan version 3: a tag after the text keeps its space",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "Done §3§."}),
            ),
            (
                "plan version 3: a removed tag between two spaces keeps one space",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "the file §4§ holds it"}),
            ),
            (
                "plan version 3: a removed tag before a full stop takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "see §3§."}),
            ),
            (
                "plan version 3: a removed tag before a comma takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "first §3§, then"}),
            ),
            (
                "plan version 3: a removed tag before a semicolon takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "one §3§; two"}),
            ),
            (
                "plan version 3: a removed tag before a colon takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "as follows §3§: this"}),
            ),
            (
                "plan version 3: a removed tag before an exclamation mark takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "done §3§!"}),
            ),
            (
                "plan version 3: a removed tag before a question mark takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "right §3§?"}),
            ),
            (
                "plan version 3: a removed tag before a closing parenthesis takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "(see §3§) next"}),
            ),
            (
                "plan version 3: a removed tag before a closing bracket takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "[see §3§] next"}),
            ),
            (
                "plan version 3: spaced tags before punctuation go with their spaces",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "x §3§ §4§."}),
            ),
            (
                "plan version 3: spaced tags between words leave one space",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "a §3§ §4§ b"}),
            ),
            (
                "plan version 3: a model-written double space stays",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "model  wrote  doubles §3§ here"}),
            ),
            (
                "plan version 3: a model-written double space before a tag stays",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "word  §3§ next"}),
            ),
            (
                "plan version 3: a tag before a newline leaves its space",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "line §3§\nnext"}),
            ),
            (
                "plan version 3: a tag after a newline leaves its space",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "line\n§3§ next"}),
            ),
            (
                "plan version 3: a tag at the very start is split off as before",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§3§ start"}),
            ),
            (
                "plan version 3: a tag at the very end is trimmed as before",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "end §3§"}),
            ),
            (
                "plan version 3: two adjacent tags between words leave both spaces",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "a §3§§4§ b"}),
            ),
            (
                "plan version 3: two adjacent tags before punctuation leave the space",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "see §3§§4§."}),
            ),
            (
                "plan version 3: a broken tag between words leaves both spaces",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "mid §11865\">§ text"}),
            ),
            (
                "plan version 3: a dangling tag before punctuation leaves its space",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "see §12$."}),
            ),
            (
                "plan version 3: tabs around a tag stay",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "a\t§3§\tb"}),
            ),
            (
                "plan version 3: non-breaking spaces around a tag stay",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "a\u{a0}§3§\u{a0}b"}),
            ),
            (
                "plan version 4 is the current rule: a tag between two spaces",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "the file §4§ holds it"}),
            ),
            (
                "plan version 4 is the current rule: a tag before punctuation",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "see §3§."}),
            ),
            (
                "plan version 4 is the current rule: adjacent tags",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "a §3§§4§ b"}),
            ),
            (
                "plan version 2 cleans as version 3 did",
                "clean_reply_for_plan",
                json!({"plan_version": 2, "text": "see §3§."}),
            ),
            // Every reply-cleaning input pinned under both plan versions, so
            // the pinned difference between 3 and 4 is exactly the space a
            // removed tag leaves.
            (
                "plan version 3: a reply's own prefix is split off",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§12§ Done."}),
            ),
            (
                "plan version 4: a reply's own prefix is split off",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§12§ Done."}),
            ),
            (
                "plan version 4: numbers inside a reply are removed",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§12§ §13§ Dropped §4§ and §5\">§6§ as asked."}),
            ),
            (
                "plan version 3: a section reference keeps its number but not its sign",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "See §5.1 of the spec."}),
            ),
            (
                "plan version 4: a section reference keeps its number but not its sign",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "See §5.1 of the spec."}),
            ),
            (
                "plan version 3: a cleaned reply is trimmed",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "  §12§ Done.\n\n"}),
            ),
            (
                "plan version 4: a cleaned reply is trimmed",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "  §12§ Done.\n\n"}),
            ),
            (
                "plan version 3: a first prefix too large for a number names no tag, and a later one never stands in",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§18446744073709551616§ §7§ Done."}),
            ),
            (
                "plan version 4: a first prefix too large for a number names no tag, and a later one never stands in",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§18446744073709551616§ §7§ Done."}),
            ),
            (
                "plan version 3: reply cleaning edge case: two leading tags",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§1§ §2§ Done."}),
            ),
            (
                "plan version 4: reply cleaning edge case: two leading tags",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§1§ §2§ Done."}),
            ),
            (
                "plan version 4: reply cleaning edge case: a tag after the text",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "Done §3§."}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section reference",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§5.1 applies"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section reference",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§5.1 applies"}),
            ),
            (
                "plan version 3: reply cleaning edge case: two section references",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§12.3 and §4"}),
            ),
            (
                "plan version 4: reply cleaning edge case: two section references",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§12.3 and §4"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a broken tag ending",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§9\"> broken"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a broken tag ending",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§9\"> broken"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a broken tag ending before a tag",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§9\">§10§ text"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a broken tag ending before a tag",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§9\">§10§ text"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section sign before a colon",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§7: note"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section sign before a colon",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§7: note"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section sign before letters",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§7a word"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section sign before letters",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§7a word"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a lone trailing section sign",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "trailing §"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a lone trailing section sign",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "trailing §"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a doubled section sign",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§§ double"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a doubled section sign",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§§ double"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a reply that is only a tag",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§1§"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a reply that is only a tag",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§1§"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a padded tag",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "  §2§  padded  "}),
            ),
            (
                "plan version 4: reply cleaning edge case: a padded tag",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "  §2§  padded  "}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section sign before a count",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§99 files"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section sign before a count",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§99 files"}),
            ),
            (
                "plan version 3: reply cleaning edge case: no section sign",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "2024 roadmap"}),
            ),
            (
                "plan version 4: reply cleaning edge case: no section sign",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "2024 roadmap"}),
            ),
            (
                "plan version 3: reply cleaning edge case: tags on separate lines",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§1§\n\n§2§ multi"}),
            ),
            (
                "plan version 4: reply cleaning edge case: tags on separate lines",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§1§\n\n§2§ multi"}),
            ),
            (
                "plan version 3: reply cleaning edge case: letters between section signs",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§x§ not a tag"}),
            ),
            (
                "plan version 4: reply cleaning edge case: letters between section signs",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§x§ not a tag"}),
            ),
            (
                "plan version 3: reply cleaning edge case: tag zero",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§0§ zero"}),
            ),
            (
                "plan version 4: reply cleaning edge case: tag zero",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§0§ zero"}),
            ),
            (
                "plan version 3: reply cleaning edge case: an oversized tag alone",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§18446744073709551616§ big"}),
            ),
            (
                "plan version 4: reply cleaning edge case: an oversized tag alone",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§18446744073709551616§ big"}),
            ),
            (
                "plan version 3: reply cleaning edge case: non-breaking spaces around a tag",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "\u{a0}§5§\u{a0}nbsp"}),
            ),
            (
                "plan version 4: reply cleaning edge case: non-breaking spaces around a tag",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "\u{a0}§5§\u{a0}nbsp"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section sign before a parenthesis",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§6) closer"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section sign before a parenthesis",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§6) closer"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section sign before a bracket",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§6] closer"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section sign before a bracket",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§6] closer"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section sign before a dash",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§6- dash"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section sign before a dash",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§6- dash"}),
            ),
            (
                "plan version 3: reply cleaning edge case: a section sign before an underscore",
                "clean_reply_for_plan",
                json!({"plan_version": 3, "text": "§6_ under"}),
            ),
            (
                "plan version 4: reply cleaning edge case: a section sign before an underscore",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§6_ under"}),
            ),
            (
                "plan version 4: a removed tag before a comma takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "first §3§, then"}),
            ),
            (
                "plan version 4: a removed tag before a semicolon takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "one §3§; two"}),
            ),
            (
                "plan version 4: a removed tag before a colon takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "as follows §3§: this"}),
            ),
            (
                "plan version 4: a removed tag before an exclamation mark takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "done §3§!"}),
            ),
            (
                "plan version 4: a removed tag before a question mark takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "right §3§?"}),
            ),
            (
                "plan version 4: a removed tag before a closing parenthesis takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "(see §3§) next"}),
            ),
            (
                "plan version 4: a removed tag before a closing bracket takes its space with it",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "[see §3§] next"}),
            ),
            (
                "plan version 4: spaced tags before punctuation go with their spaces",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "x §3§ §4§."}),
            ),
            (
                "plan version 4: spaced tags between words leave one space",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "a §3§ §4§ b"}),
            ),
            (
                "plan version 4: a model-written double space stays",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "model  wrote  doubles §3§ here"}),
            ),
            (
                "plan version 4: a model-written double space before a tag stays",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "word  §3§ next"}),
            ),
            (
                "plan version 4: a tag before a newline leaves its space",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "line §3§\nnext"}),
            ),
            (
                "plan version 4: a tag after a newline leaves its space",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "line\n§3§ next"}),
            ),
            (
                "plan version 4: a tag at the very start is split off as before",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "§3§ start"}),
            ),
            (
                "plan version 4: a tag at the very end is trimmed as before",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "end §3§"}),
            ),
            (
                "plan version 4: two adjacent tags before punctuation leave the space",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "see §3§§4§."}),
            ),
            (
                "plan version 4: a broken tag between words leaves both spaces",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "mid §11865\">§ text"}),
            ),
            (
                "plan version 4: a dangling tag before punctuation leaves its space",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "see §12$."}),
            ),
            (
                "plan version 4: tabs around a tag stay",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "a\t§3§\tb"}),
            ),
            (
                "plan version 4: non-breaking spaces around a tag stay",
                "clean_reply_for_plan",
                json!({"plan_version": 4, "text": "a\u{a0}§3§\u{a0}b"}),
            ),
        ]
    }

    /// A step with two finished calls, then the empty entries a runner can
    /// hold after it: the message carrying the step's tool results, whose
    /// results ride inside the calls, and the next step, opened but not yet
    /// filled.
    fn calls_then_empty() -> Vec<Entry> {
        vec![
            entry(1, "u1", Role::User, vec![text("t1", 1, "build it")], None),
            entry(
                2,
                "a1",
                Role::Assistant,
                vec![
                    text("t2", 2, "running both"),
                    tool("k1", 3, ToolStatus::Completed),
                    tool("k2", 4, ToolStatus::Completed),
                ],
                None,
            ),
            entry(3, "r1", Role::User, vec![], None),
            entry(4, "a2", Role::Assistant, vec![], None),
        ]
    }

    // Tool results ride in a1's calls, leaving r1 empty. Standalone system
    // deliveries follow that result message without a notice origin mark.
    fn calls_then_systems(count: u64) -> Vec<Entry> {
        let mut entries = calls_then_empty();
        entries.truncate(3);
        for number in 1..=count {
            entries.push(entry(
                number + 3,
                &format!("s{number}"),
                Role::System,
                vec![Part::new(
                    format!("st{number}"),
                    PartBody::Text {
                        text: "turn instructions".into(),
                    },
                )],
                None,
            ));
        }
        entries
    }

    #[test]
    fn pr6_system_tail_protects_both_entries_by_edit_index() {
        use crate::render::check_final_turn;

        for count in 1..=3 {
            let mut entries = calls_then_systems(count);
            entries.push(entry(4 + count, "empty", Role::Assistant, vec![], None));
            let edits = vec![drop_part(1, "a1", "k1"), drop_part(1, "a1", "k2")];
            let refusal = ViewRefusal::Rule(RuleBreak::FinalTurnRemoved { index: 1 });
            assert_eq!(check_final_turn(&entries, &edits), Err(refusal.clone()));
            assert_eq!(plan(&entries, &edits), Err(refusal));
            assert!(plan(&entries, &edits[..1]).is_ok());
            assert_eq!(
                check_final_turn(&entries, &[drop_message(1, &format!("s{count}"))]),
                Err(ViewRefusal::Rule(RuleBreak::FinalTurnRemoved { index: 0 })),
            );
        }

        let mut entries = vec![
            entry(
                1,
                "u1",
                Role::User,
                vec![text("t1", 1, "one"), text("t2", 2, "two")],
                None,
            ),
            entry(
                2,
                "s1",
                Role::System,
                vec![Part::new(
                    "st1",
                    PartBody::Text {
                        text: "instructions".into(),
                    },
                )],
                None,
            ),
        ];
        let edits = vec![drop_part(1, "u1", "t1"), drop_part(1, "u1", "t2")];
        assert!(plan(&entries, &edits[..1]).is_ok());
        assert_eq!(
            plan(&entries, &edits),
            Err(ViewRefusal::Rule(RuleBreak::FinalTurnRemoved { index: 1 })),
        );
        entries.remove(0);
        assert!(plan(&entries, &[]).is_ok());
        assert_eq!(
            plan(&entries, &[drop_message(1, "s1")]),
            Err(ViewRefusal::Rule(RuleBreak::FinalTurnRemoved { index: 0 })),
        );
    }

    /// The plan of [`transcript`] with two user messages inserted at the
    /// start, ids `m0` and `m1`, carrying these texts.
    fn head_plan(m0: &str, m1: &str) -> crate::render::RenderPlan {
        crate::render::plan(&transcript([0, 0]), &super::head(m0, m1)).unwrap()
    }

    /// Representative JSON outputs for selected rendering operations and a
    /// sample tag prefix, pinned by version, oldest first. This regression
    /// snapshot is not exhaustive and does not hash final host requests.
    /// Retaining historical pins and requiring version bumps for supported
    /// rendering changes are release-review obligations; runners persist and
    /// check the version themselves when replay compatibility is required.
    ///
    /// A row pins checksums of growing prefixes of the rendering outputs, in
    /// case order, as `(cases covered, checksum)`. The test checks the latest
    /// row only: its version must equal `PLAN_VERSION`, its counts must grow,
    /// every listed checksum must match, and its last count must cover every
    /// rendering output. Adding cases at the end of `cases()` appends a longer
    /// prefix to the latest row without touching its earlier ones, so new
    /// coverage needs no version bump. Earlier rows are kept for release review
    /// and not checked here; version 1's `usize::MAX` is a historical pin, not
    /// a coverage count.
    const PLAN_OUTPUTS: &[(u32, &[(usize, &str)])] = &[
        (
            1,
            &[(
                usize::MAX,
                "c851d76033d54c137256800577344dd5e3ee040a8f4f8cf9ece2e41bb905c665",
            )],
        ),
        (
            2,
            &[
                (
                    32,
                    "d754710059e69b97ce69ec66cca6ef9af5442a104a82d81b3a42ed6ce4332529",
                ),
                (
                    56,
                    "fa9f74315c5e6d76c949b1a337f29de1982a41ee3a88ea08eb0dc4d1b8715bd6",
                ),
            ],
        ),
        // Version 3 renders version 2's 56 cases byte for byte (same prefix
        // checksum). Plans now list a tool result's rides, and a replaced
        // output takes them with it.
        (
            3,
            &[
                (
                    56,
                    "fa9f74315c5e6d76c949b1a337f29de1982a41ee3a88ea08eb0dc4d1b8715bd6",
                ),
                (
                    61,
                    "c13b7e9fe0cf560f4a9ae6e929947b2fe83132e304006fc1895a24b0116018c0",
                ),
                // The final-turn cases: refusals, and the edits that keep the
                // request's end. Every earlier output is unchanged.
                (
                    68,
                    "e17d62170680e55235293eb35b269c3ec970f8a3e44195591b15f4a17e9c6505",
                ),
                (
                    78,
                    "b3465e2184731cd925a98d902c75149ee880061088275e4a2b5b918bd814e6fd",
                ),
                (
                    86,
                    "6aef800fa9b8366d8531b6e76bc2fdc008aa5f4c9682133e221d7dfe7b6f7f36",
                ),
                (
                    89,
                    "90c3a6bfc3bd0877b95c11c76809b0712c1575776369a4fd96c23c4a2376994f",
                ),
                // Two signed-floor cases: the floor changes which views are
                // valid, never what a valid view renders.
                (
                    91,
                    "b693d469d500e61575c1736fc58d30139cd4c0fa9b5fd7c0b3648ba161fca415",
                ),
            ],
        ),
        // Version 4: reply cleaning drops the space a removed mid-text tag
        // would leave doubled or before closing punctuation.
        (
            4,
            &[
                (
                    143,
                    "04dcfb58b5ebe5772a90a3cd15d0b28121c529c5df5ece883d309dd6a952bc87",
                ),
                // Every reply-cleaning input pinned under both plan versions.
                (
                    217,
                    "4adaad2b75e91164ada699e529991673c5666bf1ca10224e26a62fbdfae1f33c",
                ),
            ],
        ),
    ];

    /// The functions whose output a runner renders, or renders from.
    const RENDERING: &[&str] = &[
        "plan_view",
        "plan_view_at",
        "apply_with_rules",
        "plan",
        "plan_at",
        "apply",
        "edits_for_transcript",
        "assign_tags",
        "clean_reply",
        "clean_reply_for_plan",
        "cleared_ride",
    ];

    /// The rendering outputs in case order, and the checksum of the first
    /// `count` of them plus a sample tag prefix.
    fn rendered_checksum(outputs: &[&Value], count: usize) -> String {
        let prefix = json!(crate::tags::tag_prefix(12));
        let mut pinned: Vec<&Value> = outputs.iter().take(count).copied().collect();
        pinned.push(&prefix);
        cortexkit_role_compaction::digest::checksum(&pinned).to_string()
    }

    #[test]
    fn rendered_bytes_change_only_with_a_new_plan_version() {
        for pair in PLAN_OUTPUTS.windows(2) {
            assert!(pair[0].0 < pair[1].0, "versions rise: {pair:?}");
        }
        let (version, prefixes) = PLAN_OUTPUTS.last().expect("a pinned version");
        assert_eq!(*version, crate::PLAN_VERSION);
        let vectors = generate();
        let outputs: Vec<&Value> = vectors
            .iter()
            .filter(|vector| RENDERING.contains(&vector.function.as_str()))
            .map(|vector| &vector.output)
            .collect();
        for pair in prefixes.windows(2) {
            assert!(pair[0].0 < pair[1].0, "prefixes grow: {pair:?}");
        }
        for (count, checksum) in *prefixes {
            assert_eq!(
                *checksum,
                rendered_checksum(&outputs, *count),
                "checksum mismatch for the first {count} rendering cases; check the case \
                 order and the pin. If supported rendering behaviour changed, bump \
                 PLAN_VERSION and add its row to PLAN_OUTPUTS"
            );
        }
        let (covered, _) = prefixes.last().expect("a pinned prefix");
        assert_eq!(
            *covered,
            outputs.len(),
            "every rendering case is pinned: append ({}, <checksum>) to version {version}'s \
             prefixes after adding cases at the end of cases()",
            outputs.len()
        );
    }

    fn generate() -> Vec<Vector> {
        cases()
            .into_iter()
            .map(|(name, function, input)| Vector {
                name: name.into(),
                function: function.into(),
                output: call(function, input.clone()),
                input,
            })
            .collect()
    }

    #[test]
    fn the_vector_file_matches_the_library() {
        let vectors = generate();
        for vector in &vectors {
            assert!(
                FUNCTIONS.iter().any(|(name, _)| *name == vector.function),
                "{}",
                vector.function
            );
            assert!(
                vector.output.get("error").is_none(),
                "{}: {}",
                vector.name,
                vector.output
            );
        }
        // Every function has at least one case.
        for (function, _) in FUNCTIONS {
            assert!(
                vectors.iter().any(|vector| vector.function == *function),
                "no vector for {function}"
            );
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("vectors/cases.json");
        let written = serde_json::to_string_pretty(&vectors).unwrap() + "\n";
        if std::env::var_os("UPDATE_VECTORS").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &written).unwrap();
            let outputs: Vec<&Value> = vectors
                .iter()
                .filter(|vector| RENDERING.contains(&vector.function.as_str()))
                .map(|vector| &vector.output)
                .collect();
            println!(
                "rendering prefix: ({}, {:?})",
                outputs.len(),
                rendered_checksum(&outputs, outputs.len())
            );
        }
        let file = std::fs::read_to_string(&path)
            .expect("vectors/cases.json is missing; run the tests with UPDATE_VECTORS=1");
        assert_eq!(
            file, written,
            "vectors/cases.json is stale; run with UPDATE_VECTORS=1"
        );
        // Read back as a runner reads them: from JSON, each input gives its
        // output.
        let read: Vec<Vector> = serde_json::from_str(&file).unwrap();
        for vector in read {
            assert_eq!(
                call(&vector.function, vector.input),
                vector.output,
                "{}",
                vector.name
            );
        }
    }
}

#[test]
fn text_can_take_a_files_place() {
    let image = mk(
        "f1",
        None,
        Seal::None,
        PartBody::File(FileMedia::inline("image/png", Some(4096))),
    );
    let entries = vec![entry(1, "u1", Role::User, vec![image], None)];
    let edits = vec![replace(
        1,
        &entries,
        "u1",
        "f1",
        "text",
        json!("[image stripped]"),
    )];
    let rendered = apply_edits(&entries, &edits).unwrap();
    assert_eq!(
        rendered[0].parts[0].body,
        PartBody::Text {
            text: "[image stripped]".into()
        }
    );
    assert!(rendered[0].final_text.contains("f1"));
    // A file takes no other field.
    let bad = vec![replace(1, &entries, "u1", "f1", "output", json!("x"))];
    assert!(matches!(
        apply_edits(&entries, &bad),
        Err(ViewRefusal::BadField { .. })
    ));
}

mod images_and_prefixes {
    use crate::clean::split_leading;
    use crate::render::base64_decoded_len;

    #[test]
    fn the_first_prefix_is_the_models_tag_even_when_it_overflows() {
        assert_eq!(split_leading("§7§ §8§ Done"), (Some(7), "Done"));
        assert_eq!(
            split_leading("§18446744073709551616§ §7§ Done"),
            (None, "Done")
        );
        assert_eq!(split_leading("Done"), (None, "Done"));
    }

    #[test]
    fn only_standard_padded_base64_decodes() {
        assert_eq!(base64_decoded_len("YQ=="), Some(1));
        assert_eq!(base64_decoded_len("YWI="), Some(2));
        assert_eq!(base64_decoded_len("YWJj"), Some(3));
        assert_eq!(base64_decoded_len("a+/9"), Some(3));
        // `YR==` and `YWJ=` leave nonzero bits before the padding.
        for bad in [
            "", "YQ", "YQ=", "Y===", "YQ==YQ==", "-_8=", "YW J", "YQ=\n", "YR==", "YWJ=",
        ] {
            assert_eq!(base64_decoded_len(bad), None, "{bad:?}");
        }
    }
}
