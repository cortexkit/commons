use cortexkit_role_compaction_provider::{
    answer::{CompactionMessage, Coverage, Range, StepAnswer},
    geometry::Geometry,
    notice::{Channel2Directive, DispatchReceipt, NoticeLedger},
    status::{Estimate, StatusMessage, StepStatus},
    view::{
        read_frozen_view, sha256, FrozenView, RawPayload, ViewReadAnswer, ViewReadRefusal,
        ViewReadRequest,
    },
};
use cortexkit_role_step_transform_provider::claude_code::{
    ClaudeCodeCapabilities, ClaudeCodeLane, MAX_ENVELOPE_BYTES, PRINCIPAL,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/compaction-provider-v1/claude-code.json"
    ))
    .unwrap()
}
fn bundle() -> ClaudeCodeCapabilities {
    let value: Value = serde_json::from_str(include_str!(
        "../../../test-vectors/step-transform-provider-v1/claude-code.json"
    ))
    .unwrap();
    serde_json::from_value(value["bundle"].clone()).unwrap()
}
fn lane() -> ClaudeCodeLane {
    ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(bundle())).unwrap()
}
fn status() -> StepStatus {
    serde_json::from_value(vectors()["status"].clone()).unwrap()
}
fn view() -> FrozenView {
    serde_json::from_value(vectors()["replacement"]["compaction"]["raw_view"].clone()).unwrap()
}
fn request() -> ViewReadRequest {
    serde_json::from_value(vectors()["view_read"].clone()).unwrap()
}
fn bytes(view: &FrozenView) -> &str {
    let RawPayload::Inline { bytes } = &view.payload else {
        panic!("inline fixture required")
    };
    bytes
}
fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> T {
    let typed: T = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), *value);
    typed
}

#[test]
fn geometry_fields_round_trip() {
    let status: StepStatus = round_trip(&vectors()["status"]);
    assert_eq!(status.geometry.as_ref().unwrap().usable_soft, 150000);
    assert_eq!(status.output_reserve, Some(12000));
    assert_eq!(status.mid_turn, Some(true));
    assert_eq!(status.fill.as_ref().unwrap().input_tokens, 123456);
    assert_eq!(
        status.fill.as_ref().unwrap().observed_request_id.as_deref(),
        Some("provider-r0")
    );
    assert_eq!(
        status.validate_claude_code(&lane(), PRINCIPAL, "connection1"),
        Ok(())
    );
}

#[test]
fn geometry_required_only_on_attested_declared_lane() {
    for case in vectors()["geometry_refusals"].as_array().unwrap() {
        let mut status = status();
        status.geometry = serde_json::from_value(case["geometry"].clone()).unwrap();
        let refusal = status
            .validate_claude_code(&lane(), PRINCIPAL, "connection1")
            .expect_err(case["name"].as_str().unwrap());
        assert_eq!(refusal.code(), "invalid_params");
        assert_eq!(refusal.field, "geometry");
    }
}

#[test]
fn off_lane_absence_and_supplied_geometry_preserve_old_behavior() {
    for principal in [PRINCIPAL, "reserved:broca"] {
        let lane = ClaudeCodeLane::admit(principal, "connection1", None).unwrap();
        let mut status = StepStatus::new(
            "s",
            "claude-code",
            "r",
            "l",
            "step",
            "tool_step",
            "model",
            Estimate::new(1),
            1,
        );
        assert_eq!(
            status.validate_claude_code(&lane, principal, "connection1"),
            Ok(())
        );
        status.geometry = Some(Geometry {
            usable_soft: 0,
            usable_hard: 9,
            absolute_wall: 1,
            derivation: "unknown".into(),
        });
        assert_eq!(
            status.validate_claude_code(&lane, principal, "connection1"),
            Ok(())
        );
        let answer = StepAnswer::Noop {
            request_id: "r".into(),
            channel2_directive: None,
        };
        assert_eq!(
            answer.validate_claude_code(&lane, principal, "connection1"),
            Ok(())
        );
    }
}

#[test]
fn geometry_unknown_window_and_open_labels_are_accepted() {
    let mut status = status();
    status.context_window = None;
    status.geometry.as_mut().unwrap().derivation = "future-attribution".into();
    status.fill.as_mut().unwrap().basis = "future-basis".into();
    assert_eq!(
        status.validate_claude_code(&lane(), PRINCIPAL, "connection1"),
        Ok(())
    );
    status.mid_turn = Some(false);
    let wire = serde_json::to_value(&status).unwrap();
    assert_eq!(wire["mid_turn"], false);
    round_trip::<StepStatus>(&wire);
}

#[test]
fn frozen_view_and_exact_endpoint_round_trip() {
    let answer: StepAnswer = round_trip(&vectors()["replacement"]);
    assert_eq!(
        answer.validate_claude_code(&lane(), PRINCIPAL, "connection1"),
        Ok(())
    );
    let view = view();
    assert_eq!(view.validate(), Ok(()));
    assert_eq!(view.endpoint.message_id, "m0");
    assert_eq!(view.endpoint.part_index, 0);
    assert_eq!(
        sha256(bytes(&view).as_bytes()),
        "03ec95f3439527a5f74c8c011471d15f1325fa6e3c497b75518980fb44ebb6ff"
    );
    let mut reference = view.clone();
    reference.payload = RawPayload::Reference {
        view: reference.view.clone(),
    };
    assert_eq!(reference.validate(), Ok(()));
    round_trip::<FrozenView>(&serde_json::to_value(reference).unwrap());
    let endpoint = Coverage::new("mid#not-a-coordinate", u64::MAX).with_endpoint(
        cortexkit_role_compaction_provider::view::NativeEndpoint {
            message_id: "mid#not-a-coordinate".into(),
            part_index: u64::MAX,
        },
    );
    round_trip::<Coverage>(&serde_json::to_value(endpoint).unwrap());
    // Raw view retrieval is optional: the inline role replacement still applies.
    let message = CompactionMessage::new(
        "c",
        1,
        Range::new(0, 1),
        vec![json!({"role":"user","content":"head"})],
    )
    .with_raw_view(view);
    assert!(message.model_view_source().is_some());
}

#[test]
fn view_read_pages_echo_frozen_identity() {
    let view = view();
    round_trip::<ViewReadRequest>(&vectors()["view_read"]);
    let answer = read_frozen_view(
        &lane(),
        PRINCIPAL,
        "connection1",
        &view,
        bytes(&view),
        &request(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(&answer).unwrap(),
        vectors()["view_page"]
    );
    for refusal in vectors()["view_refusals"].as_array().unwrap() {
        round_trip::<ViewReadAnswer>(refusal);
    }
    let mut bundle = bundle();
    bundle.caps.view_page_bytes = 7;
    let lane = ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(bundle)).unwrap();
    let mut request = request();
    let mut collected = String::new();
    loop {
        let answer = read_frozen_view(
            &lane,
            PRINCIPAL,
            "connection1",
            &view,
            bytes(&view),
            &request,
        )
        .unwrap();
        let ViewReadAnswer::Page {
            view: echoed,
            byte_cursor,
            bytes,
            next_byte_cursor,
            ..
        } = answer
        else {
            panic!("page expected")
        };
        assert_eq!(echoed, view.view);
        assert_eq!(byte_cursor as usize, collected.len());
        collected.push_str(&bytes);
        match next_byte_cursor {
            Some(cursor) => request.byte_cursor = cursor,
            None => break,
        }
    }
    assert_eq!(collected, bytes(&view));
}

#[test]
fn view_read_refuses_digest_mismatch() {
    let view = view();
    let mut request = request();
    request.sha256 = "0".repeat(64);
    assert_eq!(
        read_frozen_view(
            &lane(),
            PRINCIPAL,
            "connection1",
            &view,
            bytes(&view),
            &request
        ),
        Err(ViewReadRefusal::DigestMismatch)
    );
}

#[test]
fn view_read_refuses_unknown_version_cursor_and_off_lane() {
    let view = view();
    let mut request = request();
    request.version = 2;
    assert_eq!(
        read_frozen_view(
            &lane(),
            PRINCIPAL,
            "connection1",
            &view,
            bytes(&view),
            &request
        ),
        Err(ViewReadRefusal::UnknownVersion)
    );
    request.version = 1;
    request.byte_cursor = u64::MAX;
    assert_eq!(
        read_frozen_view(
            &lane(),
            PRINCIPAL,
            "connection1",
            &view,
            bytes(&view),
            &request
        ),
        Err(ViewReadRefusal::InvalidCursor)
    );
    let off = ClaudeCodeLane::admit(PRINCIPAL, "connection1", None).unwrap();
    assert_eq!(
        read_frozen_view(
            &off,
            PRINCIPAL,
            "connection1",
            &view,
            bytes(&view),
            &request
        ),
        Err(ViewReadRefusal::UnavailableLane)
    );
    assert_eq!(
        read_frozen_view(
            &lane(),
            PRINCIPAL,
            "connection2",
            &view,
            bytes(&view),
            &request
        ),
        Err(ViewReadRefusal::UnavailableLane)
    );
}

#[test]
fn view_pages_account_for_escaping_and_utf8_under_shared_envelope_cap() {
    let mut view = view();
    let raw = format!("{}é", "\"\\\n".repeat(MAX_ENVELOPE_BYTES / 3));
    view.view.byte_length = raw.len() as u64;
    view.view.sha256 = sha256(raw.as_bytes());
    view.payload = RawPayload::Inline { bytes: raw.clone() };
    let mut bundle = bundle();
    bundle.caps.view_page_bytes = MAX_ENVELOPE_BYTES - 1;
    let lane = ClaudeCodeLane::admit(PRINCIPAL, "connection1", Some(bundle)).unwrap();
    let mut request = request();
    request.sha256 = view.view.sha256.clone();
    let mut collected = String::new();
    loop {
        let answer =
            read_frozen_view(&lane, PRINCIPAL, "connection1", &view, &raw, &request).unwrap();
        assert!(serde_json::to_vec(&answer).unwrap().len() < MAX_ENVELOPE_BYTES);
        let ViewReadAnswer::Page {
            bytes,
            next_byte_cursor,
            ..
        } = answer
        else {
            panic!("page expected")
        };
        collected.push_str(&bytes);
        match next_byte_cursor {
            Some(cursor) => request.byte_cursor = cursor,
            None => break,
        }
    }
    assert_eq!(collected, raw);
    request.byte_cursor = raw.len() as u64 - 1;
    assert_eq!(
        read_frozen_view(&lane, PRINCIPAL, "connection1", &view, &raw, &request),
        Err(ViewReadRefusal::InvalidCursor)
    );
}

fn directive() -> Channel2Directive {
    serde_json::from_value(vectors()["noop"]["channel2_directive"].clone()).unwrap()
}
fn receipt() -> DispatchReceipt {
    serde_json::from_value(vectors()["status"]["channel2_receipt"].clone()).unwrap()
}
fn ledger() -> NoticeLedger {
    NoticeLedger::new("synthetic-provider-generation", "l1")
}

#[test]
fn notice_directive_receipt_round_trip_and_ack_is_idempotent() {
    round_trip::<StepAnswer>(&vectors()["noop"]);
    round_trip::<Channel2Directive>(&vectors()["noop"]["channel2_directive"]);
    round_trip::<DispatchReceipt>(&vectors()["status"]["channel2_receipt"]);
    let mut ledger = ledger();
    ledger
        .arm(&lane(), PRINCIPAL, "connection1", directive())
        .unwrap();
    assert_eq!(ledger.pending(), Some(&directive()));
    assert_eq!(ledger.pending(), Some(&directive()));
    let first = ledger
        .acknowledge(&lane(), PRINCIPAL, "connection1", "notice-1", receipt())
        .unwrap();
    assert_eq!(first, receipt());
    assert!(ledger.pending().is_none());
    assert_eq!(
        ledger.acknowledge(&lane(), PRINCIPAL, "connection1", "notice-1", receipt()),
        Ok(first)
    );
}

#[test]
fn notice_ack_conflict_refuses_without_clearing_pending() {
    let mut ledger = ledger();
    ledger
        .arm(&lane(), PRINCIPAL, "connection1", directive())
        .unwrap();
    ledger
        .acknowledge(&lane(), PRINCIPAL, "connection1", "notice-1", receipt())
        .unwrap();
    let mut next = directive();
    next.directive_id = "notice-2".into();
    ledger
        .arm(&lane(), PRINCIPAL, "connection1", next.clone())
        .unwrap();
    let mut conflicting = receipt();
    conflicting.dispatch_id = "different-dispatch".into();
    let refusal = ledger
        .acknowledge(&lane(), PRINCIPAL, "connection1", "notice-1", conflicting)
        .unwrap_err();
    assert_eq!(refusal.field, "channel2_receipt");
    assert_eq!(refusal.code(), "invalid_params");
    assert_eq!(ledger.pending(), Some(&next));
}

#[test]
fn notice_ack_uses_id_and_rejects_stale_scope() {
    let mut ledger = ledger();
    ledger
        .arm(&lane(), PRINCIPAL, "connection1", directive())
        .unwrap();
    let refusal = ledger
        .acknowledge(&lane(), PRINCIPAL, "connection1", "position-0", receipt())
        .unwrap_err();
    assert_eq!(refusal.field, "channel2_delivered_id");
    for field in ["provider_generation", "lineage_id", "final_wire_sha256"] {
        let mut wire = serde_json::to_value(receipt()).unwrap();
        wire[field] = json!("wrong");
        let refusal = ledger
            .acknowledge(
                &lane(),
                PRINCIPAL,
                "connection1",
                "notice-1",
                serde_json::from_value(wire).unwrap(),
            )
            .unwrap_err();
        assert_eq!(refusal.field, "channel2_receipt");
        assert_eq!(ledger.pending(), Some(&directive()));
    }
    let refusal = ledger
        .acknowledge(&lane(), PRINCIPAL, "connection2", "notice-1", receipt())
        .unwrap_err();
    assert_eq!(refusal.field, "claude_code");
    assert_eq!(ledger.pending(), Some(&directive()));
    let mut status = status();
    status.channel2_receipt = None;
    assert_eq!(
        status
            .validate_claude_code(&lane(), PRINCIPAL, "connection1")
            .unwrap_err()
            .field,
        "channel2_receipt"
    );
}

#[test]
fn notices_are_never_armed_off_lane() {
    for principal in [PRINCIPAL, "reserved:broca"] {
        let off = ClaudeCodeLane::admit(principal, "connection1", None).unwrap();
        let mut ledger = ledger();
        assert_eq!(
            ledger
                .arm(&off, principal, "connection1", directive())
                .unwrap_err()
                .field,
            "channel2_directive"
        );
        assert!(ledger.pending().is_none());
        let answer: StepAnswer = serde_json::from_value(vectors()["noop"].clone()).unwrap();
        assert_eq!(
            answer
                .validate_claude_code(&off, principal, "connection1")
                .unwrap_err()
                .field,
            "channel2_directive"
        );
    }
}

#[test]
fn raw_view_refuses_inconsistent_endpoint_digest_and_scope() {
    for field in ["sha256", "provider_generation", "profile_generation"] {
        let mut wire = vectors()["replacement"].clone();
        wire["compaction"]["raw_view"][field] = json!("wrong");
        let answer: StepAnswer = serde_json::from_value(wire).unwrap();
        assert_eq!(
            answer
                .validate_claude_code(&lane(), PRINCIPAL, "connection1")
                .unwrap_err()
                .field,
            "raw_view"
        );
    }
    let mut wire = vectors()["replacement"].clone();
    wire["coverage"]["endpoint"]["part_index"] = json!(1);
    let answer: StepAnswer = serde_json::from_value(wire).unwrap();
    assert_eq!(
        answer
            .validate_claude_code(&lane(), PRINCIPAL, "connection1")
            .unwrap_err()
            .field,
        "raw_view"
    );
}

#[test]
fn oversized_single_message_refuses_by_name_without_truncation() {
    let mut status = status();
    let message =
        json!({"role": "user", "content": "x".repeat(bundle().caps.single_message_bytes)});
    status.messages = vec![StatusMessage {
        mid: "m0".into(),
        ordinal: 0,
        message: message.clone(),
    }];
    let before = status.messages.clone();
    let refusal = status
        .validate_claude_code(&lane(), PRINCIPAL, "connection1")
        .unwrap_err();
    assert_eq!(refusal.code(), "invalid_params");
    assert_eq!(refusal.field, "messages");
    assert_eq!(status.messages, before);
}

#[test]
fn persisted_notice_receipts_replay_after_restart() {
    let mut ledger = ledger();
    ledger
        .arm(&lane(), PRINCIPAL, "connection1", directive())
        .unwrap();
    let mut restarted: NoticeLedger =
        serde_json::from_slice(&serde_json::to_vec(&ledger).unwrap()).unwrap();
    assert_eq!(restarted.pending(), Some(&directive()));
    restarted
        .acknowledge(&lane(), PRINCIPAL, "connection1", "notice-1", receipt())
        .unwrap();
    let mut restarted: NoticeLedger =
        serde_json::from_slice(&serde_json::to_vec(&restarted).unwrap()).unwrap();
    assert_eq!(
        restarted.acknowledge(&lane(), PRINCIPAL, "connection1", "notice-1", receipt()),
        Ok(receipt())
    );
    assert!(restarted.pending().is_none());
}

#[test]
fn capability_bundle_round_trips_in_compaction_describe_and_setup() {
    use cortexkit_role_compaction_provider::{describe::RoleDescribe, setup::SetupRequest};
    let describe = RoleDescribe::new(vec![], "synthetic-build").with_claude_code(bundle());
    let setup = SetupRequest::new("s", "claude-code", "setup1", Default::default(), "model", 1)
        .with_claude_code(bundle());
    let wire = serde_json::to_value(&describe).unwrap();
    assert_eq!(wire["claude_code"], serde_json::to_value(bundle()).unwrap());
    round_trip::<RoleDescribe>(&wire);
    let wire = serde_json::to_value(&setup).unwrap();
    assert_eq!(wire["claude_code"], serde_json::to_value(bundle()).unwrap());
    let setup: SetupRequest = round_trip(&wire);
    assert_eq!(lane().check_frozen(setup.claude_code.as_ref()), Ok(()));
}

#[test]
fn new_vector_checksums_pin_both_role_fixtures() {
    for role in ["compaction-provider-v1", "step-transform-provider-v1"] {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../test-vectors")
            .join(role);
        let sums = std::fs::read_to_string(dir.join("CLAUDE_CODE_SHA256SUMS")).unwrap();
        let mut checked = 0;
        for line in sums.lines() {
            let (digest, name) = line.split_once("  ").unwrap();
            assert_eq!(
                sha256(&std::fs::read(dir.join(name)).unwrap()),
                digest,
                "{role}/{name}"
            );
            checked += 1;
        }
        assert_eq!(checked, 1);
    }
}

#[test]
fn geometry_output_reserve_must_fit_absolute_wall() {
    let mut status = status();
    status.output_reserve = Some(20000);
    assert_eq!(
        status.validate_claude_code(&lane(), PRINCIPAL, "connection1"),
        Ok(())
    );
    status.output_reserve = Some(20001);
    let refusal = status
        .validate_claude_code(&lane(), PRINCIPAL, "connection1")
        .unwrap_err();
    assert_eq!(refusal.code(), "invalid_params");
    assert_eq!(refusal.field, "geometry");
    let off = ClaudeCodeLane::admit(PRINCIPAL, "connection1", None).unwrap();
    assert_eq!(
        status.validate_claude_code(&off, PRINCIPAL, "connection1"),
        Ok(())
    );
    status.context_window = None;
    status.geometry.as_mut().unwrap().usable_hard = u64::MAX;
    status.geometry.as_mut().unwrap().absolute_wall = u64::MAX;
    status.output_reserve = Some(1);
    assert_eq!(
        status
            .validate_claude_code(&lane(), PRINCIPAL, "connection1")
            .unwrap_err()
            .field,
        "geometry"
    );
}

#[test]
fn lane_status_without_fill_is_no_reading_and_accepted() {
    let mut status = status();
    status.fill = None;
    assert!(status.geometry.is_some());
    assert_eq!(
        status.validate_claude_code(&lane(), PRINCIPAL, "connection1"),
        Ok(())
    );
    let wire = serde_json::to_value(&status).unwrap();
    assert!(wire.get("fill").is_none());
    let decoded: StepStatus = round_trip(&wire);
    assert!(decoded.fill.is_none());
    assert_eq!(
        decoded.validate_claude_code(&lane(), PRINCIPAL, "connection1"),
        Ok(())
    );
}
