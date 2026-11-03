use cortexkit_role_compaction::{
    notice::{Conditions, Event, NoticeAnswer, NoticeRequest},
    ops,
    part::{
        Attachment, AttachmentItem, Delivered, FileMedia, Opaque, Part, PartBody, Seal, ToolPart,
        ToolStatus,
    },
    read::{
        Cursor, Detail, Entry, Head, NoticeOrigin, ReadPage, ReadRequest, Role, Select, StepUsage,
    },
    ride::{RideAnswer, RideCause, RideRequest},
    rules::{EditCost, ProviderRules, RuleBreak, SignedParts},
    setup::{ModelRef, SetupAnswer, SetupRequest},
    view::{
        AppliedViewAnswer, AppliedViewRequest, ApplyViewAnswer, ApplyViewRequest, Edit, EditOp,
        InsertAt, InsertedImage, InsertedMessage, View, ViewRefusal,
    },
    wire::{self, codes, Refusal},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

fn value(value: impl Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

fn request(method: &str, params: impl Serialize) -> Value {
    serde_json::from_slice(&wire::encode(method, &params)).unwrap()
}

fn samples() -> Vec<(&'static str, Value)> {
    let model = ModelRef::new("example-provider", "example-model").with_variant("standard");
    let rules = ProviderRules::new(SignedParts::None, EditCost::Suffix);
    let conditions =
        Conditions::new(model.clone(), 32_768, 4_096, rules.clone()).with_last_prompt_tokens(1_024);
    let part = Part::new(
        "part-a",
        PartBody::Text {
            text: "An invented message.".into(),
        },
    )
    .with_tag(1);
    let tool = ToolPart::new(
        "call-a",
        "example-tool",
        json!({"query": "invented"}),
        ToolStatus::Completed,
    )
    .with_output("An invented result.")
    .with_output_bytes(19)
    .with_attachments(vec![Attachment::new(
        "attachment-a",
        AttachmentItem::File(FileMedia::inline("image/png", Some(64))),
    )])
    .with_delivered(vec![Delivered::new(
        "example-notifier",
        "An invented reminder.",
    )
    .with_template("reminder")]);
    let entry = Entry::new(1, "message-a", Role::User, 1_700_000_000_000, vec![part])
        .with_notice(NoticeOrigin::new("example-notifier").with_template("example"));
    let assistant = Entry::new(
        2,
        "message-b",
        Role::Assistant,
        1_700_000_000_100,
        vec![
            Part::new("part-b", PartBody::Tool(tool)),
            Part::new(
                "part-c",
                PartBody::Reasoning {
                    text: Some("An invented thought.".into()),
                    text_bytes: Some(20),
                },
            )
            .with_seal(Seal::Signed),
            Part::new(
                "part-d",
                PartBody::Opaque(Opaque::new("example-provider.item").with_size(32)),
            ),
        ],
    )
    .with_usage(
        StepUsage::new()
            .with_prompt_tokens(1_024)
            .with_output_tokens(64),
    );
    let cursor = Cursor::new(assistant.ordinal, &assistant.mid, &assistant.digest);
    let view = View::seal(
        1,
        vec![
            Edit::new(
                1,
                EditOp::Insert {
                    at: InsertAt::Start,
                    message: InsertedMessage::new(
                        "insert-a",
                        Role::User,
                        vec!["An invented summary.".into()],
                    ),
                },
            ),
            Edit::new(
                1,
                EditOp::Cut {
                    through: "message-a".into(),
                },
            ),
        ],
    );
    let setup = SetupRequest::new(
        "example",
        model,
        32_768,
        4_096,
        rules,
        vec![RideCause::CacheExpired, RideCause::OverLimit],
        1_700_000_000_200,
    )
    .with_newest(cursor.clone())
    .with_applied_view(view.applied());
    let mut compact_assistant = assistant.clone();
    compact_assistant.parts = assistant.parts.iter().map(Part::compact).collect();
    let notice = NoticeRequest::new(1, conditions.clone())
        .with_newest(cursor.clone())
        .with_entries(vec![entry.clone(), compact_assistant])
        .with_events(vec![Event::MarkerWritten { version: 1 }])
        .with_applied_view(view.applied());
    let empty_notice = NoticeRequest::new(2, conditions.clone())
        .with_events(vec![Event::TranscriptChanged { from_ordinal: 1 }]);
    let ride = RideRequest::new(vec![RideCause::OverLimit], 1_500, conditions)
        .with_request_tokens(30_000)
        .with_applied_view(view.applied());
    let image_message = InsertedMessage::new(
        "insert-image",
        Role::User,
        vec!["An invented visual summary.".into()],
    )
    .with_images(vec![InsertedImage::new(
        "image/png",
        "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAAC0lEQVR4nGNgAAIAAAUAAXpeqz8AAAAASUVORK5CYII=",
    )]);
    let image_request = |message| {
        ApplyViewRequest::new(View::seal(
            1,
            vec![Edit::new(
                1,
                EditOp::Insert {
                    at: InsertAt::Start,
                    message,
                },
            )],
        ))
    };
    let mut samples = vec![
        (
            "setup-accepts-images-request.json",
            request(
                ops::COMPACTION_SETUP,
                setup.clone().with_accepts_images(true),
            ),
        ),
        ("setup-request.json", request(ops::COMPACTION_SETUP, setup)),
        ("setup-answer.json", value(SetupAnswer::Ready)),
        (
            "notice-request.json",
            request(ops::COMPACTION_NOTICE, notice),
        ),
        ("notice-answer.json", value(NoticeAnswer::Received)),
        (
            "notice-empty-transcript-request.json",
            request(ops::COMPACTION_NOTICE, empty_notice),
        ),
        (
            "notice-setup-required.json",
            value(NoticeAnswer::SetupRequired),
        ),
        ("ride-request.json", request(ops::COMPACTION_RIDE, ride)),
        (
            "ride-answer.json",
            value(RideAnswer::View { view: view.clone() }),
        ),
        ("ride-nothing.json", value(RideAnswer::Nothing)),
        (
            "entries-request.json",
            request(
                ops::SESSION_ENTRIES,
                ReadRequest::new(
                    Select::After {
                        after_ordinal: None,
                        limit: Some(10),
                    },
                    Detail::Compact,
                )
                .with_max_bytes(65_536),
            ),
        ),
        (
            "entries-answer.json",
            value(ReadPage::new(
                vec![entry, assistant],
                Head::new()
                    .with_newest(cursor)
                    .with_applied_view(view.applied()),
            )),
        ),
        (
            "apply-view-request.json",
            request(ops::SESSION_APPLY_VIEW, ApplyViewRequest::new(view.clone())),
        ),
        (
            "apply-view-image-request.json",
            request(
                ops::SESSION_APPLY_VIEW,
                image_request(image_message.clone()),
            ),
        ),
        ("apply-view-answer.json", value(ApplyViewAnswer::Accepted)),
        (
            "apply-view-refused.json",
            value(ApplyViewAnswer::Refused {
                refusal: ViewRefusal::BadChecksum,
            }),
        ),
        (
            "applied-view-request.json",
            request(ops::SESSION_APPLIED_VIEW, AppliedViewRequest::new()),
        ),
        (
            "applied-view-answer.json",
            value(AppliedViewAnswer::new().with_view(view)),
        ),
        ("applied-view-none.json", value(AppliedViewAnswer::new())),
    ];
    let mut assistant_image = image_message.clone();
    assistant_image.role = Role::Assistant;
    let mut non_png_image = image_message;
    non_png_image.images[0].media_type = "image/jpeg".into();
    for (name, message) in [
        ("image-assistant-refused.json", assistant_image),
        ("image-non-png-refused.json", non_png_image),
    ] {
        samples.push((
            name,
            json!({
                "request": request(ops::SESSION_APPLY_VIEW, image_request(message)),
                "answer": ApplyViewAnswer::Refused {
                    refusal: ViewRefusal::UnsupportedEdit { index: 0 },
                },
            }),
        ));
    }
    let refusals = [
        ("bad-checksum", ViewRefusal::BadChecksum),
        (
            "stale-version",
            ViewRefusal::StaleVersion {
                version: 1,
                latest: 2,
            },
        ),
        ("bad-since", ViewRefusal::BadSince),
        (
            "unknown-message",
            ViewRefusal::UnknownMessage {
                mid: "message-missing".into(),
            },
        ),
        (
            "unknown-part",
            ViewRefusal::UnknownPart {
                mid: "message-a".into(),
                part: "part-missing".into(),
            },
        ),
        (
            "stale-part",
            ViewRefusal::StalePart {
                mid: "message-a".into(),
                part: "part-a".into(),
            },
        ),
        (
            "bad-field",
            ViewRefusal::BadField {
                mid: "message-a".into(),
                part: "part-a".into(),
                field: "output".into(),
            },
        ),
        (
            "conflict",
            ViewRefusal::Conflict {
                mid: "message-a".into(),
                part: "part-a".into(),
            },
        ),
        (
            "running-tool",
            ViewRefusal::RunningTool {
                mid: "message-b".into(),
                part: "part-b".into(),
            },
        ),
        (
            "unknown-anchor",
            ViewRefusal::UnknownAnchor {
                mid: "message-missing".into(),
            },
        ),
        (
            "duplicate-insert",
            ViewRefusal::DuplicateInsert {
                id: "insert-a".into(),
            },
        ),
        ("duplicate-cut", ViewRefusal::DuplicateCut),
        (
            "tagged-insert",
            ViewRefusal::TaggedInsert {
                id: "insert-a".into(),
            },
        ),
        (
            "foreign-tag",
            ViewRefusal::ForeignTag {
                mid: "message-a".into(),
                part: "part-a".into(),
                field: "text".into(),
                tag: 9,
            },
        ),
        (
            "rule",
            ViewRefusal::Rule(RuleBreak::SignedPartInvalid {
                mid: "message-b".into(),
                part: "part-c".into(),
            }),
        ),
        (
            "unsupported_edit",
            ViewRefusal::UnsupportedEdit { index: 3 },
        ),
    ];
    let names = [
        "view-bad-checksum.json",
        "view-stale-version.json",
        "view-bad-since.json",
        "view-unknown-message.json",
        "view-unknown-part.json",
        "view-stale-part.json",
        "view-bad-field.json",
        "view-conflict.json",
        "view-running-tool.json",
        "view-unknown-anchor.json",
        "view-duplicate-insert.json",
        "view-duplicate-cut.json",
        "view-tagged-insert.json",
        "view-foreign-tag.json",
        "view-rule.json",
        "view-unsupported-edit.json",
    ];
    for (name, (_, refusal)) in names.into_iter().zip(refusals) {
        samples.push((name, value(refusal)));
    }
    samples.extend([
        (
            "refusal-invalid-params.json",
            value(Refusal::invalid_params(
                "context_window",
                "expected an unsigned integer",
            )),
        ),
        (
            "refusal-unknown-method.json",
            value(Refusal::unknown_method("session.example")),
        ),
        (
            "refusal-unknown-preset.json",
            value(Refusal::unknown_preset("example-missing")),
        ),
        (
            "refusal-unknown-session.json",
            value(Refusal::unknown_session()),
        ),
        (
            "refusal-unattested-runner.json",
            value(Refusal::new(
                codes::UNATTESTED_RUNNER,
                "No caller module attested",
                None,
            )),
        ),
        (
            "refusal-unknown-route-role.json",
            value(Refusal::new(
                codes::UNKNOWN_ROUTE_ROLE,
                "Expected exactly one served role",
                None,
            )),
        ),
        (
            "refusal-route-unbound.json",
            value(Refusal::new(
                codes::ROUTE_UNBOUND,
                "Expected a bound session",
                None,
            )),
        ),
    ]);
    samples.push((
        "entries-max-integer-request.json",
        request(
            ops::SESSION_ENTRIES,
            ReadRequest::new(
                Select::After {
                    after_ordinal: Some(9_007_199_254_740_991),
                    limit: Some(1),
                },
                Detail::Compact,
            ),
        ),
    ));
    for (name, select, field, message) in [
        (
            "validation-integer-too-large.json",
            json!({"by": "after", "after_ordinal": 9_007_199_254_740_992_u64}),
            "select.after_ordinal",
            "integer exceeds 9007199254740991",
        ),
        (
            "validation-zero-limit.json",
            json!({"by": "after", "limit": 0}),
            "select.limit",
            "limit must be at least 1",
        ),
    ] {
        samples.push((name, json!({
            "request": {"method": ops::SESSION_ENTRIES, "params": {"select": select, "detail": "compact"}},
            "refusal": Refusal::invalid_params(field, message),
        })));
    }
    let entry = Entry::new(
        1,
        "message-tool",
        Role::Assistant,
        1_700_000_000_000,
        vec![Part::new(
            "part-tool",
            PartBody::Tool(ToolPart::new(
                "call-tool",
                "example-tool",
                json!({}),
                ToolStatus::Running,
            )),
        )],
    );
    let without_empty = value(&entry);
    let mut with_empty = without_empty.clone();
    with_empty["parts"][0]["attachments"] = json!([]);
    samples.push((
        "checksum-empty-attachments.json",
        json!({
            "with_empty": with_empty,
            "without_empty": without_empty,
            "checksum": entry.digest,
        }),
    ));
    samples
}

fn round_trip<T: DeserializeOwned + Serialize>(json: Value) {
    let decoded: T = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(value(decoded), json);
}

fn request_round_trip<T: DeserializeOwned + Serialize>(
    method: &str,
    params: Value,
    expected: Value,
) {
    let decoded: T = wire::params(params).unwrap();
    let reencoded: Value = serde_json::from_slice(&wire::encode(method, &decoded)).unwrap();
    assert_eq!(reencoded, expected);
}

fn check(name: &str, json: Value) {
    if name.starts_with("image-") {
        // These pairs specify the runner's refusal, not a wire decoding error:
        // edit validation is performed by the view library.
        let (method, params) =
            wire::decode(&serde_json::to_vec(&json["request"]).unwrap()).unwrap();
        assert_eq!(method, ops::SESSION_APPLY_VIEW);
        request_round_trip::<ApplyViewRequest>(&method, params, json["request"].clone());
        round_trip::<ApplyViewAnswer>(json["answer"].clone());
    } else if name.starts_with("validation-") {
        let (method, params) =
            wire::decode(&serde_json::to_vec(&json["request"]).unwrap()).unwrap();
        assert_eq!(method, ops::SESSION_ENTRIES);
        let refusal = wire::params::<ReadRequest>(params).unwrap_err();
        assert_eq!(value(refusal), json["refusal"]);
    } else if name == "checksum-empty-attachments.json" {
        assert_eq!(json["with_empty"]["parts"][0]["attachments"], json!([]));
        assert!(json["without_empty"]["parts"][0]
            .get("attachments")
            .is_none());
        for form in ["with_empty", "without_empty"] {
            let entry: Entry = serde_json::from_value(json[form].clone()).unwrap();
            assert_eq!(
                cortexkit_role_compaction::read::entry_digest(
                    entry.role,
                    entry.notice.as_ref(),
                    &entry.parts
                ),
                json["checksum"].as_str().unwrap()
            );
        }
    } else if name.ends_with("-request.json") {
        let (method, params) = wire::decode(&serde_json::to_vec(&json).unwrap()).unwrap();
        match method.as_str() {
            ops::COMPACTION_SETUP => request_round_trip::<SetupRequest>(&method, params, json),
            ops::COMPACTION_NOTICE => request_round_trip::<NoticeRequest>(&method, params, json),
            ops::COMPACTION_RIDE => request_round_trip::<RideRequest>(&method, params, json),
            ops::SESSION_ENTRIES => request_round_trip::<ReadRequest>(&method, params, json),
            ops::SESSION_APPLY_VIEW => {
                request_round_trip::<ApplyViewRequest>(&method, params, json)
            }
            ops::SESSION_APPLIED_VIEW => {
                request_round_trip::<AppliedViewRequest>(&method, params, json)
            }
            _ => panic!("unrecognized vector method"),
        }
    } else if name.starts_with("view-") {
        round_trip::<ViewRefusal>(json);
    } else if name.starts_with("refusal-") {
        round_trip::<Refusal>(json);
    } else if name.starts_with("setup-") {
        round_trip::<SetupAnswer>(json);
    } else if name.starts_with("notice-") {
        round_trip::<NoticeAnswer>(json);
    } else if name.starts_with("ride-") {
        round_trip::<RideAnswer>(json);
    } else if name.starts_with("entries-") {
        round_trip::<ReadPage>(json);
    } else if name.starts_with("apply-view-") {
        round_trip::<ApplyViewAnswer>(json);
    } else if name.starts_with("applied-view-") {
        round_trip::<AppliedViewAnswer>(json);
    } else {
        panic!("unrecognized vector name: {name}");
    }
}

#[test]
fn wire_vectors_round_trip() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/compaction-v2");
    for (name, generated) in samples() {
        if std::env::var_os("UPDATE_COMPACTION_VECTORS").is_some() {
            std::fs::create_dir_all(&root).unwrap();
            let path = root.join(name);
            let bytes = format!("{}\n", serde_json::to_string_pretty(&generated).unwrap());
            if std::fs::read(&path).ok().as_deref() != Some(bytes.as_bytes()) {
                std::fs::write(path, bytes).unwrap();
            }
        }
        let saved: Value =
            serde_json::from_slice(&std::fs::read(root.join(name)).unwrap()).unwrap();
        assert_eq!(
            saved, generated,
            "{name}: regenerate vectors after an intentional wire change"
        );
        check(name, saved);
    }
}
