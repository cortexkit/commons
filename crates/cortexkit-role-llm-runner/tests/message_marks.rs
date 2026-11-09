use cortexkit_role_llm_runner::{
    display::DisplayRow,
    history::HistoryEntry,
    live::NoticeDelivered,
    message_marks::{agent_id_valid, MessageMarks, SpeaksFor},
    read::{NoticeFact, ReadMessage},
    send::SendRequest,
};
use serde_json::{json, Value};

fn request(marks: Value) -> SendRequest {
    let mut raw = json!({"prompt":"hello", "send_id":"send-a"});
    raw.as_object_mut()
        .unwrap()
        .extend(marks.as_object().unwrap().clone());
    serde_json::from_value(raw).unwrap()
}

#[test]
fn message_marks_admission_names_fields_and_trusts_only_the_scope_owner() {
    let speaks = request(json!({"speaks_for":{"agent_id":"agent-a"}}));
    let marks = speaks
        .message_marks(true, Some("module-x"), "module-x")
        .unwrap();
    assert_eq!(marks.speaks_for, Some(SpeaksFor::new("agent-a")));
    assert!(!marks.notice);
    assert_eq!(
        speaks.message_marks(true, None, "module-x"),
        Err("speaks_for")
    );
    assert_eq!(
        speaks.message_marks(true, Some("module-x"), "module-y"),
        Err("speaks_for")
    );
    assert_eq!(
        speaks.message_marks(false, Some("module-x"), "module-x"),
        Err("speaks_for")
    );
    let both = request(json!({"speaks_for":{"agent_id":"agent-a"},"notice":true}));
    assert_eq!(
        both.message_marks(true, Some("module-x"), "module-x"),
        Err("notice")
    );
    assert_eq!(both.message_marks(true, None, "module-y"), Err("notice"));
    for owner in [None, Some("module-x")] {
        assert!(
            request(json!({"notice":true}))
                .message_marks(true, owner, "module-y")
                .unwrap()
                .notice
        );
    }
    for raw in [json!(null), json!("true"), json!(1), json!({})] {
        assert_eq!(
            request(json!({"notice":raw})).message_marks(true, None, "module-x"),
            Err("notice")
        );
    }
    for notice in [true, false] {
        assert_eq!(
            request(json!({"notice":notice})).message_marks(false, None, "module-x"),
            Err("notice")
        );
    }
    assert!(
        request(json!({"speaks_for":{"agent_id":"agent-a"},"notice":false}))
            .message_marks(true, Some("module-x"), "module-x")
            .is_ok()
    );
}

#[test]
fn message_marks_agent_ids_are_one_to_128_printable_ascii_bytes() {
    for id in ["agent-a".to_owned(), " ".to_owned(), "~".repeat(128)] {
        assert!(agent_id_valid(&id));
        assert!(request(json!({"speaks_for":{"agent_id":id}}))
            .message_marks(true, Some("module-x"), "module-x")
            .is_ok());
    }
    for id in [
        "".to_owned(),
        "a".repeat(129),
        "a\n".to_owned(),
        "\u{7f}".to_owned(),
        "é".to_owned(),
    ] {
        assert!(!agent_id_valid(&id));
        assert_eq!(
            request(json!({"speaks_for":{"agent_id":id}})).message_marks(
                true,
                Some("module-x"),
                "module-x"
            ),
            Err("speaks_for.agent_id")
        );
    }
    for raw in [
        json!(null),
        json!({}),
        json!({"agent_id":42}),
        json!("agent-a"),
    ] {
        assert_eq!(
            request(json!({"speaks_for":raw})).message_marks(true, Some("module-x"), "module-x"),
            Err("speaks_for.agent_id")
        );
    }
    assert_eq!(
        request(json!({"speaks_for":{"agent_id":"agent-a","extra":true}})).message_marks(
            true,
            Some("module-x"),
            "module-x"
        ),
        Err("speaks_for")
    );
}

#[test]
fn notice_with_speaks_for_checks_names_and_keeps_owner_admission() {
    for name in [" ".to_owned(), "Ada".to_owned(), "~".repeat(64)] {
        let request =
            request(json!({"speaks_for":{"agent_id":"agent-a", "name":name}, "notice":true}));
        let marks = request
            .message_marks_with_notice_speaks_for(true, true, Some("owner"), "owner")
            .unwrap();
        assert!(marks.notice);
        assert_eq!(
            marks.speaks_for,
            Some(SpeaksFor::new("agent-a").with_name(name))
        );
        assert_eq!(
            request.message_marks_with_notice_speaks_for(true, false, Some("owner"), "owner"),
            Err("notice")
        );
        assert_eq!(
            request.message_marks_with_notice_speaks_for(true, true, Some("owner"), "stranger"),
            Err("speaks_for")
        );
        assert_eq!(
            request.message_marks_with_notice_speaks_for(true, true, None, "owner"),
            Err("speaks_for")
        );
        assert_eq!(
            request.message_marks_with_notice_speaks_for(false, true, Some("owner"), "owner"),
            Err("speaks_for")
        );
    }
    for name in [
        json!(""),
        json!("a".repeat(65)),
        json!("é"),
        json!("\u{7f}"),
        json!("a\t"),
        json!(null),
        json!(1),
    ] {
        assert_eq!(
            request(json!({"speaks_for":{"agent_id":"agent-a", "name":name}}))
                .message_marks_with_notice_speaks_for(true, true, Some("owner"), "owner"),
            Err("speaks_for.name")
        );
    }
    let named = request(json!({"speaks_for":{"agent_id":"agent-a", "name":"Ada"}}));
    assert_eq!(
        named.message_marks(true, Some("owner"), "owner"),
        Err("speaks_for.name")
    );
    let unnamed = serde_json::to_value(SpeaksFor::new("agent-a")).unwrap();
    assert!(unnamed.get("name").is_none());
}

#[test]
fn message_marks_unmarked_send_bytes_are_unchanged_and_false_is_omitted() {
    let bytes =
        r#"{"prompt":"hello","send_id":"send-a","delivery":"queue","model":{"id":"model-a"}}"#;
    let send: SendRequest = serde_json::from_str(bytes).unwrap();
    assert_eq!(serde_json::to_string(&send).unwrap(), bytes);
    assert_eq!(
        send.message_marks(false, None, "module-x").unwrap(),
        MessageMarks::default()
    );
    let false_send = send.clone().with_notice(false);
    assert_eq!(serde_json::to_string(&false_send).unwrap(), bytes);
    let decoded_false: SendRequest =
        serde_json::from_str(&bytes.replace("\"model\"", "\"notice\":false,\"model\"")).unwrap();
    assert_eq!(serde_json::to_string(&decoded_false).unwrap(), bytes);
    let marked = SendRequest::new("hello", "send-a").with_speaks_for(SpeaksFor::new("agent-a"));
    let raw = serde_json::to_value(&marked).unwrap();
    assert_eq!(raw["speaks_for"], json!({"agent_id":"agent-a"}));
    let decoded: SendRequest = serde_json::from_value(raw).unwrap();
    assert_eq!(
        decoded.message_marks(true, Some("module-x"), "module-x"),
        marked.message_marks(true, Some("module-x"), "module-x")
    );
}

#[test]
fn message_marks_round_trip_beside_unchanged_bodies_on_all_surfaces() {
    for notice in [false, true] {
        let body = json!({"role":"user", "content":"hello"});
        let mut read = ReadMessage::new(1, "mid-a", body.clone())
            .with_written_at(10)
            .with_sender("module-x");
        read.notice = notice;
        if !notice {
            read.speaks_for = Some(SpeaksFor::new("agent-a"));
        }
        let raw = serde_json::to_value(&read).unwrap();
        let decoded: ReadMessage = serde_json::from_value(raw.clone()).unwrap();
        let history = HistoryEntry::from_read(decoded).unwrap();
        let mut row = DisplayRow::new("user", vec![]).with_notice(notice);
        row.speaks_for = read.speaks_for.clone();
        for surface in [
            serde_json::to_value(history).unwrap(),
            serde_json::to_value(row).unwrap(),
        ] {
            assert_eq!(surface.get("notice"), raw.get("notice"));
            assert_eq!(surface.get("speaks_for"), raw.get("speaks_for"));
        }
        assert_eq!(raw["message"], body);
        assert_eq!(
            raw.get("notice"),
            if notice {
                Some(&Value::Bool(true))
            } else {
                None
            }
        );
        assert_eq!(serde_json::from_value::<ReadMessage>(raw).unwrap(), read);
    }
}

#[test]
fn message_marks_riding_facts_and_events_have_only_the_agent_label() {
    let fact = NoticeFact::new(0, "send-a", "module-x", "hello")
        .with_call_key("call-a")
        .with_speaks_for(SpeaksFor::new("agent-a"));
    let event = NoticeDelivered::new("mid-a", 0, "call-a", "send-a", "module-x")
        .with_text("hello")
        .with_speaks_for(SpeaksFor::new("agent-a"));
    let raw_fact = serde_json::to_value(&fact).unwrap();
    let raw_event = serde_json::to_value(&event).unwrap();
    for raw in [&raw_fact, &raw_event] {
        assert_eq!(raw["speaks_for"], json!({"agent_id":"agent-a"}));
        assert!(raw.get("notice").is_none());
    }
    assert_eq!(
        serde_json::from_value::<NoticeFact>(raw_fact).unwrap(),
        fact
    );
    assert_eq!(
        serde_json::from_value::<NoticeDelivered>(raw_event).unwrap(),
        event
    );
}

#[test]
fn named_message_marks_round_trip_on_riding_and_stored_surfaces() {
    let mark = SpeaksFor::new("agent-a").with_name("Ada");
    let mut read = ReadMessage::new(
        1,
        "mid-a",
        json!({"role":"user", "content":"carried words"}),
    )
    .with_written_at(10);
    read.speaks_for = Some(mark.clone());
    read.notice = true;
    let mut row = DisplayRow::new("user", vec![]).with_notice(true);
    row.speaks_for = Some(mark.clone());
    let history = HistoryEntry::from_read(read.clone()).unwrap();
    let fact =
        NoticeFact::new(0, "send-a", "module-x", "carried words").with_speaks_for(mark.clone());
    let event =
        NoticeDelivered::new("mid-a", 0, "call-a", "send-a", "module-x").with_speaks_for(mark);
    for surface in [
        serde_json::to_value(&read).unwrap(),
        serde_json::to_value(&history).unwrap(),
        serde_json::to_value(&row).unwrap(),
        serde_json::to_value(&fact).unwrap(),
        serde_json::to_value(&event).unwrap(),
    ] {
        assert_eq!(
            surface["speaks_for"],
            json!({"agent_id":"agent-a", "name":"Ada"})
        );
    }
    assert_eq!(
        serde_json::from_value::<ReadMessage>(serde_json::to_value(&read).unwrap()).unwrap(),
        read
    );
    assert_eq!(
        serde_json::from_value::<HistoryEntry>(serde_json::to_value(&history).unwrap()).unwrap(),
        history
    );
    assert_eq!(
        serde_json::from_value::<DisplayRow>(serde_json::to_value(&row).unwrap()).unwrap(),
        row
    );
    assert_eq!(
        serde_json::from_value::<NoticeFact>(serde_json::to_value(&fact).unwrap()).unwrap(),
        fact
    );
    assert_eq!(
        serde_json::from_value::<NoticeDelivered>(serde_json::to_value(&event).unwrap()).unwrap(),
        event
    );
}
