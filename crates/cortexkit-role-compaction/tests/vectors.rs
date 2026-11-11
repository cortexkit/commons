use cortexkit_role_compaction::{
    notice::{Conditions, Event, NoticeAnswer, NoticeRequest},
    ops,
    part::{
        Attachment, AttachmentItem, Delivered, FileMedia, Opaque, Part, PartBody, RideForm, Seal,
        ToolPart, ToolStatus,
    },
    read::{
        Cursor, Detail, Entry, Head, NoticeOrigin, ReadName, ReadPage, ReadRequest, Role, Select,
        SpeaksFor, StepUsage,
    },
    ride::{RideAnswer, RideCause, RideRequest},
    rules::{EditCost, ProviderRules, RuleBreak, SignedParts},
    setup::{Limits, ModelRef, PageLimits, SetupAnswer, SetupRequest, WindowLimits},
    view::{
        AppliedViewAnswer, AppliedViewRequest, ApplyViewAnswer, ApplyViewRequest, Edit, EditOp,
        InsertAt, InsertedImage, InsertedMessage, View, ViewRefusal,
    },
    wire::{self, codes, not_permitted_reasons, Refusal},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

mod support;

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
    let ride = RideRequest::new(vec![RideCause::OverLimit], 1_500, conditions.clone())
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
            "setup-limits-request.json",
            request(
                ops::COMPACTION_SETUP,
                setup
                    .clone()
                    .with_limits(
                        Limits::new()
                            .with_page(
                                PageLimits::new()
                                    .with_max_entries(512)
                                    .with_max_bytes(1_048_576),
                            )
                            .with_select_max(256)
                            .with_window(
                                WindowLimits::new()
                                    .with_entries(8_000)
                                    .with_bytes(67_108_864),
                            ),
                    )
                    .with_window_over_budget_detection(true, true),
            ),
        ),
        (
            "setup-limits-legacy-request.json",
            request(
                ops::COMPACTION_SETUP,
                setup
                    .clone()
                    .with_limits(
                        Limits::new()
                            .with_page(
                                PageLimits::new()
                                    .with_max_entries(512)
                                    .with_max_bytes(1_048_576),
                            )
                            .with_select_max(256)
                            .with_window(
                                WindowLimits::new()
                                    .with_entries(8_000)
                                    .with_bytes(67_108_864),
                            ),
                    )
                    .with_window_over_budget_detection(true, false),
            ),
        ),
        (
            "notice-window-over-budget-request.json",
            request(
                ops::COMPACTION_NOTICE,
                NoticeRequest::new(5, conditions.clone().with_window_over_budget(true, true)),
            ),
        ),
        (
            "ride-window-over-budget-request.json",
            request(
                ops::COMPACTION_RIDE,
                RideRequest::new(vec![], 1_500, conditions.clone())
                    .with_window_over_budget(true, true),
            ),
        ),
        (
            "entries-unanswered-mids.json",
            value(
                ReadPage::new(vec![entry.clone()], Head::new())
                    .with_unanswered(vec![ReadName::Mid("message-b".into())], true),
            ),
        ),
        (
            "entries-unanswered-tags.json",
            value(
                ReadPage::new(vec![entry.clone()], Head::new())
                    .with_unanswered(vec![ReadName::Tag(2)], true),
            ),
        ),
        (
            "refusal-select-too-many.json",
            value(Refusal::select_too_many(256)),
        ),
        (
            "setup-statement-seq-request.json",
            request(
                ops::COMPACTION_SETUP,
                setup
                    .clone()
                    .with_statement_seq(42, true)
                    .with_signed_floor(
                        cortexkit_role_compaction::SignedFloor::new("message-a"),
                        true,
                    ),
            ),
        ),
        (
            "notice-statement-seq-request.json",
            request(
                ops::COMPACTION_NOTICE,
                NoticeRequest::new(
                    5,
                    conditions
                        .clone()
                        .with_statement_seq(42, true)
                        .with_signed_floor(
                            cortexkit_role_compaction::SignedFloor::new("message-a"),
                            true,
                        )
                        .with_last_prompt_view(1)
                        .with_system_tokens(cortexkit_role_compaction::tokens::SurfaceTokens::new(
                            100, true,
                        ))
                        .with_tools_tokens(cortexkit_role_compaction::tokens::SurfaceTokens::new(
                            50, true,
                        ))
                        .with_notes_tokens(
                            cortexkit_role_compaction::tokens::SurfaceTokens::new(25, true),
                            true,
                        ),
                ),
            ),
        ),
        (
            "ride-statement-seq-request.json",
            request(
                ops::COMPACTION_RIDE,
                RideRequest::new(
                    vec![RideCause::OverLimit],
                    1_500,
                    conditions
                        .clone()
                        .with_statement_seq(42, true)
                        .with_signed_floor(
                            cortexkit_role_compaction::SignedFloor::new("message-a"),
                            true,
                        )
                        .with_last_prompt_view(1),
                ),
            ),
        ),
        (
            "setup-statement-seq-restored-request.json",
            request(
                ops::COMPACTION_SETUP,
                setup.clone().with_statement_seq(7, true),
            ),
        ),
        (
            "notice-notes-declared-request.json",
            request(
                ops::COMPACTION_NOTICE,
                NoticeRequest::new(
                    5,
                    conditions
                        .clone()
                        .with_system_tokens(cortexkit_role_compaction::tokens::SurfaceTokens::new(
                            100, true,
                        ))
                        .with_tools_tokens(cortexkit_role_compaction::tokens::SurfaceTokens::new(
                            50, true,
                        ))
                        .with_notes_tokens(
                            cortexkit_role_compaction::tokens::SurfaceTokens::new(25, true),
                            true,
                        ),
                )
                .with_token_family("example-family/1"),
            ),
        ),
        (
            "notice-notes-legacy-request.json",
            request(
                ops::COMPACTION_NOTICE,
                NoticeRequest::new(
                    5,
                    conditions
                        .clone()
                        .with_system_tokens(cortexkit_role_compaction::tokens::SurfaceTokens::new(
                            125, true,
                        ))
                        .with_tools_tokens(cortexkit_role_compaction::tokens::SurfaceTokens::new(
                            50, true,
                        ))
                        .with_notes_tokens(
                            cortexkit_role_compaction::tokens::SurfaceTokens::new(25, true),
                            false,
                        ),
                )
                .with_token_family("example-family/1"),
            ),
        ),
        (
            "speaks-for-entry.json",
            value(
                Entry::new(
                    1,
                    "message-a",
                    Role::User,
                    1_700_000_000_000,
                    entry.parts.clone(),
                )
                .with_speaks_for(SpeaksFor::new("agent-a").with_name("Ada"), true),
            ),
        ),
        (
            "notice-speaks-for-entry.json",
            value(
                entry
                    .clone()
                    .with_speaks_for(SpeaksFor::new("agent-a").with_name("Ada"), true),
            ),
        ),
        (
            "notice-last-prompt-view-request.json",
            request(
                ops::COMPACTION_NOTICE,
                NoticeRequest::new(3, conditions.clone().with_last_prompt_view(7))
                    .with_applied_view(View::seal(7, view.edits.clone()).applied()),
            ),
        ),
        (
            "notice-last-prompt-no-view-request.json",
            request(
                ops::COMPACTION_NOTICE,
                NoticeRequest::new(4, conditions.clone().with_last_prompt_view(0)),
            ),
        ),
        (
            "notice-keep-entry.json",
            value(
                entry.clone().with_notice(
                    NoticeOrigin::new("example-notifier")
                        .with_template("example")
                        .with_keep(true),
                ),
            ),
        ),
        (
            "setup-accepts-images-request.json",
            request(
                ops::COMPACTION_SETUP,
                setup.clone().with_accepts_images(true),
            ),
        ),
        (
            "setup-child-harness-request.json",
            request(
                ops::COMPACTION_SETUP,
                setup.clone().with_child_harness("example-child-host"),
            ),
        ),
        (
            "applied-view-attributed.json",
            value(
                AppliedViewAnswer::new()
                    .with_view(view.clone())
                    .with_applied_by("example-provider"),
            ),
        ),
        (
            "apply-view-signed-policy-request.json",
            request(
                ops::SESSION_APPLY_VIEW,
                ApplyViewRequest::new(View::seal_with_policy(
                    2,
                    view.edits.clone(),
                    Some("message-b".into()),
                    Some("strip".into()),
                )),
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
            "cut-before-move",
            ViewRefusal::CutBeforeMove {
                moved_through: "message-a".into(),
            },
        ),
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
        "view-cut-before-move.json",
        "view-tagged-insert.json",
        "view-foreign-tag.json",
        "view-rule.json",
        "view-unsupported-edit.json",
    ];
    for (name, (_, refusal)) in names.into_iter().zip(refusals) {
        samples.push((name, value(refusal)));
    }
    samples.push((
        "view-rule-final-turn.json",
        value(ViewRefusal::Rule(RuleBreak::FinalTurnRemoved { index: 4 })),
    ));
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
        (
            "refusal-compaction-not-permitted.json",
            value(Refusal::compaction_not_permitted(
                not_permitted_reasons::PROVIDER_MISMATCH,
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
    let entry = Entry::new(
        1,
        "message-finished",
        Role::Assistant,
        1_700_000_000_000,
        vec![Part::new(
            "part-finished",
            PartBody::Text {
                text: "An invented reply.".into(),
            },
        )
        .with_tag(1)],
    );
    samples.push((
        "checksum-completed-at.json",
        json!({
            "without_completed_at": entry,
            "with_completed_at": entry.clone().with_completed_at(1_700_000_060_000),
            "checksum": entry.digest,
        }),
    ));
    samples.push(("entries-token-counts.json", value(support::counted_page())));
    samples.push((
        "notice-token-counts-request.json",
        request(ops::COMPACTION_NOTICE, support::counted_notice()),
    ));
    let delivered = Delivered::new("example-notifier", "An invented reminder.");
    let ride_entry = |delivered| {
        Entry::new(
            1,
            "ride-message",
            Role::Assistant,
            1000,
            vec![Part::new(
                "ride-part",
                PartBody::Tool(
                    ToolPart::new(
                        "ride-call",
                        "example-tool",
                        json!({}),
                        ToolStatus::Completed,
                    )
                    .with_output("An invented result.")
                    .with_delivered(vec![delivered]),
                ),
            )],
        )
    };
    let current = delivered
        .clone()
        .with_send_id("send-a")
        .with_forms(vec![RideForm::TurnSystem, RideForm::ToolResult]);
    samples.push(("delivered-rides.json", json!({
        "legacy": ride_entry(delivered),
        "current": ride_entry(current.clone()),
        "counted": ride_entry(current.with_tokens(cortexkit_role_compaction::tokens::TokenCount::new(5))),
    })));
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
    if matches!(
        name,
        "speaks-for-entry.json" | "notice-speaks-for-entry.json"
    ) {
        round_trip::<Entry>(json.clone());
        let entry: Entry = serde_json::from_value(json).unwrap();
        let mark = entry.speaks_for.clone().unwrap();
        assert_eq!(mark, SpeaksFor::new("agent-a").with_name("Ada"));
        assert_eq!(
            entry.notice.is_some(),
            name == "notice-speaks-for-entry.json"
        );
        let base = Entry::new(
            entry.ordinal,
            &entry.mid,
            entry.role.clone(),
            entry.written_at,
            entry.parts.clone(),
        );
        let base = match entry.notice.clone() {
            Some(notice) => base.with_notice(notice),
            None => base,
        };
        // These pins predate attribution: adding it must not change entry identity.
        assert_eq!(entry.digest, base.digest);
        if entry.notice.is_some() {
            assert_eq!(
                entry.digest,
                "e4565946a06648fa4ba6a87d56934d4506449a4e850ef6a5f588e3375f1e5ea7"
            );
        }
        for declared in [false, true] {
            let provides = if declared {
                vec![
                    cortexkit_role_compaction::COMPACTION_PROVIDES.into(),
                    cortexkit_role_compaction::COMPACTION_ENTRY_SPEAKS_FOR_PROVIDES.into(),
                ]
            } else {
                vec![cortexkit_role_compaction::COMPACTION_PROVIDES.into()]
            };
            let projected = base.clone().with_speaks_for(
                mark.clone(),
                cortexkit_role_compaction::provides_entry_speaks_for(&provides),
            );
            assert_eq!(projected.speaks_for.as_ref(), declared.then_some(&mark));
            assert_eq!(projected.digest, base.digest);
            assert_eq!(value(projected).get("speaks_for").is_some(), declared);
        }
        return;
    }
    if name == "notice-keep-entry.json" {
        round_trip::<Entry>(json.clone());
        let mut entry: Entry = serde_json::from_value(json.clone()).unwrap();
        assert!(entry.notice.as_ref().unwrap().keep);
        assert_eq!(
            entry.digest,
            cortexkit_role_compaction::read::entry_digest(
                entry.role.clone(),
                entry.notice.as_ref(),
                &entry.parts
            )
        );
        entry.notice.as_mut().unwrap().keep = false;
        assert!(value(entry.notice.as_ref().unwrap()).get("keep").is_none());
        // This digest belongs to the pre-keep notice-request vector. Keeping the
        // literal proves false does not silently change an existing entry's hash.
        let legacy = "e4565946a06648fa4ba6a87d56934d4506449a4e850ef6a5f588e3375f1e5ea7";
        assert_eq!(
            cortexkit_role_compaction::read::entry_digest(
                entry.role.clone(),
                entry.notice.as_ref(),
                &entry.parts
            ),
            legacy
        );
        assert_ne!(entry.digest, legacy);
        let mut explicit_false = value(&entry);
        explicit_false["notice"]["keep"] = json!(false);
        let decoded: Entry = serde_json::from_value(explicit_false).unwrap();
        assert_eq!(
            cortexkit_role_compaction::read::entry_digest(
                decoded.role,
                decoded.notice.as_ref(),
                &decoded.parts
            ),
            legacy
        );
    } else if name == "delivered-rides.json" {
        for form in ["legacy", "current", "counted"] {
            round_trip::<Entry>(json[form].clone());
            let entry: Entry = serde_json::from_value(json[form].clone()).unwrap();
            assert_eq!(
                entry.digest,
                cortexkit_role_compaction::read::entry_digest(
                    entry.role,
                    entry.notice.as_ref(),
                    &entry.parts
                )
            );
        }
        assert!(json["legacy"]["parts"][0]["delivered"][0]
            .get("send_id")
            .is_none());
        assert!(json["legacy"]["parts"][0]["delivered"][0]
            .get("forms")
            .is_none());
        assert_eq!(json["current"]["digest"], json["counted"]["digest"]);
    } else if name.starts_with("image-") {
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
    } else if name == "checksum-completed-at.json" {
        assert!(json["without_completed_at"].get("completed_at").is_none());
        assert_eq!(
            json["with_completed_at"]["completed_at"],
            1_700_000_060_000_u64
        );
        let mut without_stamp = json["with_completed_at"].clone();
        without_stamp
            .as_object_mut()
            .unwrap()
            .remove("completed_at");
        assert_eq!(without_stamp, json["without_completed_at"]);
        for form in ["with_completed_at", "without_completed_at"] {
            round_trip::<Entry>(json[form].clone());
            let entry: Entry = serde_json::from_value(json[form].clone()).unwrap();
            assert_eq!(entry.digest, json["checksum"].as_str().unwrap());
            assert_eq!(
                cortexkit_role_compaction::read::entry_digest(
                    entry.role,
                    entry.notice.as_ref(),
                    &entry.parts
                ),
                json["checksum"].as_str().unwrap()
            );
        }
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
        let bytes = std::fs::read(root.join(name)).unwrap();
        let saved: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            saved, generated,
            "{name}: regenerate vectors after an intentional wire change"
        );
        // Fixtures use sorted JSON object keys, pretty printing and one newline.
        // Preserve those bytes as well as the decoded wire value.
        assert_eq!(
            bytes,
            format!("{}\n", serde_json::to_string_pretty(&generated).unwrap()).as_bytes(),
            "{name}: fixture bytes changed"
        );
        check(name, saved);
    }
}
