use cortexkit_role_compaction_provider::{
    lineage::MessageRef,
    notice::ViewReady,
    setup::{SetupRequest, VIEW_FEATURE_TAG_BYTES_EDITS, VIEW_FEATURE_TRUNCATE},
    view::{
        dispose_fetched, CompactionView, PartContainer, PartEdit, TranscriptMessage, TruncateMode,
        ViewDisposition, ViewProblem, ViewRange,
    },
};
use serde_json::{json, Map};

fn view(edit: PartEdit) -> CompactionView {
    CompactionView::new(
        "0123456789abcdef0123456789abcdef",
        1,
        "lineage",
        MessageRef {
            ordinal: 0,
            mid: "mid".into(),
        },
        vec![ViewRange::keep_with_edits(0, 1, vec![edit])],
    )
}

#[test]
fn setup_view_features_round_trip_and_omission() {
    let request = SetupRequest::new("s", "test", "setup", Map::new(), "m", 1);
    assert!(request.view_features().is_empty());
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        r#"{"session":"s","harness":"test","request_id":"setup","params":{},"composition":{},"model":"m","now":1}"#
    );
    let request = request.with_view_features(vec![
        VIEW_FEATURE_TRUNCATE.into(),
        VIEW_FEATURE_TAG_BYTES_EDITS.into(),
        "future".into(),
    ]);
    let decoded: SetupRequest =
        serde_json::from_value(serde_json::to_value(&request).unwrap()).unwrap();
    assert_eq!(
        decoded.view_features(),
        &["truncate", "tag_bytes_edits", "future"]
    );
    assert_eq!(decoded, request);
}

#[test]
fn part_edits_require_exactly_one_action() {
    for bytes in [false, true] {
        for drop in [false, true] {
            for truncate in [false, true] {
                for attachments in [false, true] {
                    let mut edit = PartEdit::drop(0, "mid", "p");
                    edit.drop = drop;
                    edit.bytes = bytes.then(|| "{}".into());
                    edit.truncate = truncate.then_some(TruncateMode::Skeleton);
                    edit.drop_attachments = attachments;
                    let valid = u8::from(bytes) + u8::from(drop) + u8::from(truncate) == 1
                        && (!attachments || truncate);
                    assert_eq!(
                        view(edit).check(),
                        if valid {
                            Ok(())
                        } else {
                            Err(ViewProblem::EditAction)
                        }
                    );
                }
            }
        }
    }
    let edit = PartEdit::truncate(0, "mid", "p", TruncateMode::Placeholder).with_drop_attachments();
    assert_eq!(
        edit.check_container(PartContainer::Other),
        Err(ViewProblem::EditAction)
    );
    assert_eq!(edit.check_container(PartContainer::ToolResult), Ok(()));
}

#[test]
fn truncate_digest_frames_are_explicit_and_legacy_bytes_unchanged() {
    // Fixed independent SHA-256 answers use the contract's length-prefixed frames.
    let cases = [
        (
            PartEdit::replace(0, "mid", "p", "{}"),
            "bc43a81cfbb530dfe7898a9d597eaae362b730235aa92e4de978649c3b0a9d28",
        ),
        (
            PartEdit::drop(0, "mid", "p"),
            "ca15aaf9f5abc1480ab48ada8d03fa84f5f8ee0103db6be770994307ff821d51",
        ),
        (
            PartEdit::truncate(0, "mid", "p", TruncateMode::Skeleton),
            "2aa90adad5a03d2503d3ab224fd93eebd79d3535a1fa716a7be9d260a32e6dfc",
        ),
        (
            PartEdit::truncate(0, "mid", "p", TruncateMode::Skeleton).with_drop_attachments(),
            "1cb682108a170117ae2650e2583086eb61750a4358746e3f37222b606af93bf2",
        ),
    ];
    for (edit, expected) in cases {
        let value = view(edit);
        assert_eq!(value.compute_digest(), expected);
        let decoded: CompactionView =
            serde_json::from_value(serde_json::to_value(&value).unwrap()).unwrap();
        assert_eq!(decoded, value);
    }
    assert_eq!(
        serde_json::to_value(PartEdit::replace(0, "mid", "p", "{}")).unwrap(),
        json!({"ordinal":0,"mid":"mid","part":"p","bytes":"{}"})
    );
}

#[test]
fn unknown_truncate_mode_refuses_instead_of_rejecting_json() {
    let edit: PartEdit =
        serde_json::from_value(json!({"ordinal":0,"mid":"mid","part":"p","truncate":"future"}))
            .unwrap();
    let view = view(edit);
    assert_eq!(view.check(), Ok(()));
    let ready = ViewReady::for_view(&view);
    assert_eq!(
        dispose_fetched(&view, &ready, None, "lineage", 0, |_| Some(
            TranscriptMessage::new(
                0,
                "mid",
                vec!["p".into()],
                json!({"content":[{"type":"text","text":"original"}]})
            )
        )),
        ViewDisposition::Refuse("truncate_kind")
    );
}

#[test]
fn pi_text_placeholder_and_separate_image_drop_preserve_siblings_and_error_fields() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-provider-v1/pi-part-edits.json"
    ))
    .unwrap();
    let original = &fixture["original"];
    let edits: Vec<PartEdit> = serde_json::from_value(fixture["edits"].clone()).unwrap();
    let keys = vec![
        "text".to_owned(),
        "image-1".to_owned(),
        "image-2".to_owned(),
    ];
    let mut rendered = Vec::new();
    for edit in edits {
        edit.check_container(PartContainer::ToolResult).unwrap();
        if let Some(mode) = &edit.truncate {
            let index = keys.iter().position(|key| *key == edit.part).unwrap();
            let value = cortexkit_role_llm_runner::tags::truncate_part(
                "pi",
                &original["message"]["content"][index],
                PartContainer::ToolResult,
                mode.clone(),
                None,
                edit.drop_attachments,
            )
            .unwrap();
            rendered.push(PartEdit::replace(
                edit.ordinal,
                edit.mid,
                edit.part,
                serde_json::to_string(&value).unwrap(),
            ));
        } else {
            rendered.push(edit);
        }
    }
    let assembled = CompactionView::new(
        "0123456789abcdef0123456789abcdef",
        1,
        "lineage",
        MessageRef {
            ordinal: 0,
            mid: "result-row".into(),
        },
        vec![ViewRange::keep_with_edits(0, 1, rendered)],
    )
    .assemble(&[TranscriptMessage::new(
        0,
        "result-row",
        keys,
        original["message"].clone(),
    )])
    .unwrap();
    let mut actual = original.clone();
    actual["message"] = assembled[0].clone();
    assert_eq!(actual, fixture["expected"]);
}
