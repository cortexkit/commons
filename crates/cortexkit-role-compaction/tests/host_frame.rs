use cortexkit_role_compaction::{
    part::{body_digest, Part, PartBody},
    provides_host_frame,
    read::{entry_digest, Entry, HostFrame, Role},
    view::{Edit, EditOp, View},
    Text, COMPACTION_HOST_FRAME_PROVIDES, COMPACTION_PROVIDES,
};
use cortexkit_role_llm_runner::{
    attribution::host_frames::{
        CLAUDE_CODE_MID_TURN, CLAUDE_CODE_MID_TURN_HEAD, CLAUDE_CODE_MID_TURN_TAIL,
    },
    vectors,
};
use serde_json::{json, Value};

const FRAME_PART: &str = "steer-text";

fn stored_text() -> String {
    let stored: Value = serde_json::from_str(
        vectors::file("steer-frame-claude-code-v1/steer-4/stored-message.json").unwrap(),
    )
    .unwrap();
    stored["content"].as_str().unwrap().to_owned()
}

fn fact() -> HostFrame {
    HostFrame::new(
        FRAME_PART,
        CLAUDE_CODE_MID_TURN,
        CLAUDE_CODE_MID_TURN_HEAD.len() as u32,
        CLAUDE_CODE_MID_TURN_TAIL.len() as u32,
    )
}

fn entry(role: Role, text: impl Into<Text>) -> Entry {
    Entry::new(
        16,
        "steer-message",
        role,
        1_795_515_411_791,
        vec![
            Part::new(
                "another-part",
                PartBody::Text {
                    text: "Not the framed part.".into(),
                },
            ),
            Part::new(FRAME_PART, PartBody::Text { text: text.into() }).with_tag(20),
        ],
    )
}

#[test]
fn host_frame_round_trips_on_user_and_system_entries() {
    for role in [Role::User, Role::System] {
        let plain = entry(role, stored_text());
        let wire = serde_json::to_value(&plain).unwrap();
        assert!(wire.get("host_frame").is_none());
        assert_eq!(serde_json::from_value::<Entry>(wire).unwrap(), plain);

        let marked = plain.with_host_frame(fact(), true).unwrap();
        let wire = serde_json::to_value(&marked).unwrap();
        assert_eq!(
            wire["host_frame"],
            json!({
                "part": FRAME_PART,
                "code": CLAUDE_CODE_MID_TURN,
                "head_bytes": CLAUDE_CODE_MID_TURN_HEAD.len(),
                "tail_bytes": CLAUDE_CODE_MID_TURN_TAIL.len(),
            })
        );
        assert_eq!(serde_json::from_value::<Entry>(wire).unwrap(), marked);
    }
}

#[test]
fn host_frame_decode_and_memory_checks_refuse_invalid_facts() {
    fn assert_refused(mut entry: Entry, fact: HostFrame, reason: &str) {
        assert_eq!(
            entry.clone().with_host_frame(fact.clone(), true),
            Err(reason)
        );
        entry.host_frame = Some(fact);
        assert_eq!(entry.check_host_frame(), Err(reason));
        let error =
            serde_json::from_str::<Entry>(&serde_json::to_string(&entry).unwrap()).unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
    }

    for role in [Role::Assistant, Role::Unknown("future_role".into())] {
        assert_refused(
            entry(role, "text"),
            fact(),
            "host_frame is allowed only on a user or system entry",
        );
    }

    let mut missing = fact();
    missing.part = "not-a-part".into();
    assert_refused(
        entry(Role::System, "text"),
        missing,
        "host_frame names an id not in the entry",
    );

    let mut non_text = entry(Role::User, "text");
    non_text.parts[1].body = PartBody::Reasoning {
        text: None,
        text_bytes: None,
    };
    assert_refused(non_text, fact(), "host_frame must name a text part");

    for code in [
        "".into(),
        "X".into(),
        "a-b".into(),
        "é".into(),
        "a".repeat(65),
    ] {
        let mut bad_code = fact();
        bad_code.code = code;
        assert_refused(
            entry(Role::System, "text"),
            bad_code,
            "host_frame code must be 1-64 bytes of lowercase ASCII letters, digits or _",
        );
    }
}

#[test]
fn host_frame_decode_refuses_null_and_multiple_frames() {
    let plain = entry(Role::System, "text");
    let frame = fact();
    for invalid in [
        Value::Null,
        json!([frame, frame]),
        json!([frame.part, frame.code, frame.head_bytes, frame.tail_bytes]),
    ] {
        let mut wire = serde_json::to_value(&plain).unwrap();
        wire["host_frame"] = invalid;
        assert!(serde_json::from_value::<Entry>(wire).is_err());
    }

    let bare = serde_json::to_string(&plain).unwrap();
    let frame = serde_json::to_string(&frame).unwrap();
    let duplicate = format!(
        r#"{},"host_frame":{frame},"host_frame":{frame}}}"#,
        bare.strip_suffix('}').unwrap()
    );
    let error = serde_json::from_str::<Entry>(&duplicate).unwrap_err();
    assert!(error.to_string().contains("duplicate field `host_frame`"));
}

#[test]
fn host_frame_provider_gate_clears_previously_marked_entries() {
    assert_eq!(COMPACTION_HOST_FRAME_PROVIDES, "compaction-host-frame/v1");
    let plain = entry(Role::System, stored_text());
    let marked = plain.clone().with_host_frame(fact(), true).unwrap();
    for (capabilities, declared) in [
        (vec![], false),
        (vec![COMPACTION_PROVIDES.into()], false),
        (vec!["compaction-host-frame/v2".into()], false),
        (vec!["compaction-host-frame/v1-extra".into()], false),
        (
            vec![
                COMPACTION_PROVIDES.into(),
                COMPACTION_HOST_FRAME_PROVIDES.into(),
            ],
            true,
        ),
    ] {
        assert_eq!(provides_host_frame(&capabilities), declared);
        let projected = marked
            .clone()
            .with_host_frame(fact(), provides_host_frame(&capabilities))
            .unwrap();
        assert_eq!(&projected, if declared { &marked } else { &plain });
        assert_eq!(
            serde_json::to_value(projected)
                .unwrap()
                .get("host_frame")
                .is_some(),
            declared
        );
    }
}

#[test]
fn host_frame_words_keep_the_copy_of_the_head_inside_the_persons_words() {
    let expected = format!("{CLAUDE_CODE_MID_TURN_HEAD}hello");
    let marked = entry(Role::System, stored_text())
        .with_host_frame(fact(), true)
        .unwrap();
    let mut compact = marked.clone();
    compact.parts = compact.parts.iter().map(Part::compact).collect();
    for entry in [marked, compact] {
        let decoded: Entry = serde_json::from_slice(&serde_json::to_vec(&entry).unwrap()).unwrap();
        let frame = decoded.host_frame.as_ref().unwrap();
        assert_eq!(frame.words(&decoded), Some(expected.as_str()));
    }
}

#[test]
fn host_frame_words_refuse_mismatched_bytes_counts_and_unknown_codes() {
    let text = stored_text();
    let mut bad_head = text.clone();
    bad_head.replace_range(..1, "?");
    let mut bad_tail = text.clone();
    bad_tail.replace_range(text.len() - 1.., "?");
    for bad_text in [bad_head, bad_tail] {
        assert_eq!(fact().words(&entry(Role::System, bad_text)), None);
    }

    let mut bad_head_count = fact();
    bad_head_count.head_bytes += 1;
    let mut bad_tail_count = fact();
    bad_tail_count.tail_bytes -= 1;
    let mut unknown_code = fact();
    unknown_code.code = "future_frame_2".into();
    let plain = entry(Role::System, text);
    for unverified in [bad_head_count, bad_tail_count, unknown_code] {
        let marked = plain.clone().with_host_frame(unverified, true).unwrap();
        let decoded: Entry = serde_json::from_value(serde_json::to_value(marked).unwrap()).unwrap();
        assert_eq!(decoded.host_frame.as_ref().unwrap().words(&decoded), None);
    }
}

#[test]
fn host_frame_leaves_all_w7_checksums_and_stored_content_unchanged() {
    let plain = entry(Role::System, stored_text());
    let marked = plain.clone().with_host_frame(fact(), true).unwrap();
    let mut wire = serde_json::to_value(&marked).unwrap();
    assert!(wire.as_object_mut().unwrap().remove("host_frame").is_some());
    assert_eq!(wire, serde_json::to_value(&plain).unwrap());

    assert_eq!(marked.digest, plain.digest);
    assert_eq!(
        entry_digest(marked.role.clone(), marked.notice.as_ref(), &marked.parts),
        plain.digest
    );
    for (marked_part, plain_part) in marked.parts.iter().zip(&plain.parts) {
        assert_eq!(marked_part.digest, plain_part.digest);
        assert_eq!(
            body_digest(marked_part.seal.clone(), &marked_part.body),
            plain_part.digest
        );
    }

    fn seal_view(entry: &Entry) -> View {
        View::seal_with_policy(
            1,
            vec![Edit::new(
                1,
                EditOp::Replace {
                    mid: entry.mid.clone(),
                    part: entry.parts[1].id.clone(),
                    field: "text".into(),
                    value: json!("shortened"),
                    expect: entry.parts[1].digest.clone(),
                },
            )],
            Some(entry.mid.clone()),
            None,
        )
    }
    assert_eq!(seal_view(&marked).checksum, seal_view(&plain).checksum);
}

fn list_vector(name: &str) -> Value {
    let name = format!("steer-frame-claude-code-v2/{name}");
    serde_json::from_str(vectors::file(&name).unwrap()).unwrap()
}

#[test]
fn host_frames_compaction_entry_and_provider_gate_match_the_samples() {
    use cortexkit_role_compaction::{provides_host_frames, COMPACTION_HOST_FRAMES_PROVIDES};
    assert_eq!(COMPACTION_HOST_FRAMES_PROVIDES, "compaction-host-frame/v2");
    let wire = list_vector("compaction-entry.json");
    let marked: Entry = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(json!(marked), wire);
    let frames = marked.host_frames().to_vec();
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.words(&marked).unwrap())
            .collect::<Vec<_>>(),
        ["Try the blue dial. ", "Leave the brass latch open.",]
    );
    assert_eq!(
        entry_digest(marked.role.clone(), None, &marked.parts),
        marked.digest
    );
    assert!(marked
        .parts
        .iter()
        .all(|part| body_digest(part.seal.clone(), &part.body) == part.digest));
    for capabilities in list_vector("compaction-gates.json").as_array().unwrap() {
        let provides: Vec<String> =
            serde_json::from_value(capabilities["provides"].clone()).unwrap();
        let declared = provides_host_frames(&provides);
        assert_eq!(json!(declared), capabilities["host_frames"]);
        for detail in [marked.clone(), {
            let mut compact = marked.clone();
            compact.parts = compact.parts.iter().map(Part::compact).collect();
            compact
        }] {
            let projected = detail.with_host_frames(frames.clone(), declared).unwrap();
            let projected_wire = json!(projected);
            assert_eq!(projected_wire.get("host_frames").is_some(), declared);
            assert!(projected_wire.get("host_frame").is_none());
            assert_eq!(projected.digest, marked.digest);
            assert_eq!(
                serde_json::from_value::<Entry>(projected_wire).unwrap(),
                projected
            );
        }
    }
    assert!(!provides_host_frames(&[
        "compaction-host-frame/v2-extra".into()
    ]));
    let old = marked
        .clone()
        .with_host_frames(vec![], true)
        .unwrap()
        .with_host_frame(frames[1].clone(), true)
        .unwrap();
    assert_eq!(old.host_frames(), &frames[1..]);
    let read_legacy: Entry = serde_json::from_value(json!(old)).unwrap();
    assert_eq!(read_legacy.host_frames(), &frames[1..]);
    assert_eq!(old.with_host_frames(frames, true).unwrap(), marked);
}

#[test]
fn host_frames_compaction_refuses_invalid_wire_and_in_memory_lists() {
    fn assert_refused(entry: Entry, reason: &str) {
        assert_eq!(entry.check_host_frames(), Err(reason));
        let error = serde_json::from_value::<Entry>(json!(entry)).unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
    }
    let wire = list_vector("compaction-entry.json");
    let entry: Entry = serde_json::from_value(wire.clone()).unwrap();
    let mut invalid = entry.clone();
    invalid.role = Role::Assistant;
    assert_refused(
        invalid,
        "host_frame is allowed only on a user or system entry",
    );
    let mut invalid = entry.clone();
    invalid.host_frames = Some(vec![]);
    assert_refused(invalid, "host_frames must not be empty");
    let mut invalid = entry.clone();
    invalid.parts[0].body = PartBody::Reasoning {
        text: None,
        text_bytes: None,
    };
    assert_refused(invalid, "host_frame must name a text part");
    let mut invalid = entry.clone();
    invalid.host_frames.as_mut().unwrap()[0].part = "not-a-part".into();
    assert_refused(invalid, "host_frame names an id not in the entry");
    let mut invalid = entry.clone();
    invalid.host_frames.as_mut().unwrap()[0].code = "Bad_Code".into();
    assert_refused(
        invalid,
        "host_frame code must be 1-64 bytes of lowercase ASCII letters, digits or _",
    );
    let mut invalid = entry.clone();
    invalid.host_frames.as_mut().unwrap().reverse();
    assert_refused(
        invalid,
        "host_frames must be unique and in stored part order",
    );
    let mut invalid = entry.clone();
    invalid.host_frames = Some(vec![entry.host_frames()[0].clone(); 2]);
    assert_refused(
        invalid,
        "host_frames must be unique and in stored part order",
    );
    let mut invalid = entry.clone();
    invalid.host_frame = Some(entry.host_frames()[0].clone());
    assert_refused(invalid, "host_frame and host_frames are mutually exclusive");
    for value in [
        Value::Null,
        wire["host_frames"][0].clone(),
        json!([["z-first", "claude_code_mid_turn", 52, 233]]),
    ] {
        let mut invalid = wire.clone();
        invalid["host_frames"] = value;
        assert!(serde_json::from_value::<Entry>(invalid).is_err());
    }
}

#[test]
fn host_frames_compaction_verifies_items_independently() {
    let mut wire = list_vector("compaction-entry.json");
    wire["host_frames"][0]["code"] = json!("future_code");
    let entry: Entry = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(entry.host_frames()[0].words(&entry), None);
    assert_eq!(
        entry.host_frames()[1].words(&entry),
        Some("Leave the brass latch open.")
    );
    wire["host_frames"][0]["code"] = json!(CLAUDE_CODE_MID_TURN);
    wire["host_frames"][1]["tail_bytes"] = json!(233);
    let entry: Entry = serde_json::from_value(wire).unwrap();
    assert_eq!(
        entry.host_frames()[0].words(&entry),
        Some("Try the blue dial. ")
    );
    assert_eq!(entry.host_frames()[1].words(&entry), None);
}
