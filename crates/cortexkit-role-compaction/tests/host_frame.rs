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
