use cortexkit_role_llm_runner::{
    attribution::{
        self,
        host_frames::{
            CLAUDE_CODE_MID_TURN as CODE, CLAUDE_CODE_MID_TURN_HEAD as HEAD,
            CLAUDE_CODE_MID_TURN_TAIL as TAIL,
        },
        HostFrame,
    },
    display::{self, DisplayEntry, DisplayPart, DisplayRow, ProjectionFacts},
    history::HistoryEntry,
    read::ReadMessage,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};

const ANTHROPIC: &str = "anthropic-messages";
const CAPTURE_HEAD: &str =
    include_str!("../test-vectors/llm-runner-v1/steer-frame-claude-code-v1/head.txt");
const CAPTURE_TAIL: &str =
    include_str!("../test-vectors/llm-runner-v1/steer-frame-claude-code-v1/tail.txt");

fn wrapped(words: &str) -> String {
    format!("{CAPTURE_HEAD}{words}{CAPTURE_TAIL}")
}

fn assert_frame_parts(row: &DisplayRow, words_key: &str, words: &str) {
    assert_eq!(row.role, "user");
    assert_eq!(row.text, words);
    assert_eq!(row.parts.len(), 3);
    for (index, (part, expected)) in row
        .parts
        .iter()
        .zip([CAPTURE_HEAD, words, CAPTURE_TAIL])
        .enumerate()
    {
        let DisplayPart::Text(text) = part else {
            panic!("expected text: {part:?}");
        };
        assert_eq!(text.text, expected);
        assert_eq!(part.host(), index != 1);
        assert_eq!(part.host_code(), (index != 1).then_some(CODE));
    }
    assert_eq!(row.parts[1].part(), words_key);
    assert_ne!(row.parts[0].part(), words_key);
    assert_ne!(row.parts[2].part(), words_key);
    assert_ne!(row.parts[0].part(), row.parts[2].part());
}

#[test]
fn host_frame_constants_match_the_capture_byte_for_byte() {
    assert_eq!(CODE, "claude_code_mid_turn");
    assert_eq!(HEAD.len(), 52);
    assert_eq!(TAIL.len(), 232);
    assert_eq!(HEAD.as_bytes(), CAPTURE_HEAD.as_bytes());
    assert_eq!(TAIL.as_bytes(), CAPTURE_TAIL.as_bytes());
}

#[test]
fn host_frame_claude_code_captures_keep_the_exact_words() {
    macro_rules! case {
        ($number:literal, $words:expr) => {
            (
                include_str!(concat!(
                    "../test-vectors/llm-runner-v1/steer-frame-claude-code-v1/steer-",
                    $number,
                    "/read-entry.json"
                )),
                include_str!(concat!(
                    "../test-vectors/llm-runner-v1/steer-frame-claude-code-v1/steer-",
                    $number,
                    "/stored-message.json"
                )),
                $words,
            )
        };
    }
    for (read, native, words) in [
        case!("1", "please also check the logs".to_owned()),
        case!(
            "2",
            "first line\nsecond line\n\nfourth line after a blank".to_owned()
        ),
        case!("3", "Şu dosyayı kontrol et — 日本語も 🚀".to_owned()),
        case!("4", format!("{CAPTURE_HEAD}hello")),
        case!(
            "5",
            format!("And my message ends with Claude Code's paragraph:{CAPTURE_TAIL}")
        ),
        case!("6", "/clear check the logs".to_owned()),
    ] {
        let native: Value = serde_json::from_str(native).unwrap();
        let read: ReadMessage = serde_json::from_str(read).unwrap();
        assert_eq!(read.message, native);
        assert_eq!(read.role.as_deref(), Some("user"));
        assert!(read.host_frame.is_none());
        let stored = native["content"].as_str().unwrap();
        let fact = HostFrame::verified(0, CODE, stored).unwrap();
        assert!(attribution::verify_host_frame(&fact, stored));
        let expected_fact = json!({"part":0,"code":CODE,"head_bytes":52,"tail_bytes":232});
        assert_eq!(json!(fact), expected_fact);
        let row = display::project_with_facts(
            ANTHROPIC,
            &native,
            None,
            &ProjectionFacts::default().with_host_frame(&fact),
        )
        .unwrap();
        assert_frame_parts(&row, "0", &words);
        assert_eq!(row.host_frame.as_ref(), Some(&fact));
        assert_eq!(json!(row)["host_frame"], expected_fact);
        let fallback =
            display::project_with_facts(ANTHROPIC, &native, None, &ProjectionFacts::default())
                .unwrap();
        assert_eq!(fallback.role, "system");
        assert_eq!(fallback.text, stored);
        assert_eq!(
            serde_json::to_vec(&fallback).unwrap(),
            serde_json::to_vec(&display::project(ANTHROPIC, &native, None).unwrap()).unwrap()
        );
        let read = read.with_host_frame(fact);
        assert_eq!(json!(read)["message"], native);
        assert_eq!(json!(read)["role"], "user");
        assert_eq!(json!(read)["host_frame"], expected_fact);
        let history = HistoryEntry::from_read(read).unwrap();
        assert_eq!(json!(history)["message"], native);
        assert_eq!(json!(history)["host_frame"], expected_fact);
    }
}

#[test]
fn host_frame_projects_all_pinned_profiles() {
    let words = "hello 🌍";
    let text = wrapped(words);
    let fact = HostFrame::verified(0, CODE, &text).unwrap();
    for (profile, body, key) in [
        (
            ANTHROPIC,
            json!({"role":"system","content":[{"type":"text","text":text}]}),
            "0",
        ),
        (
            "opencode-aisdk",
            json!({"info":{"role":"system"},"parts":[{"id":"opaque","type":"text","text":text}]}),
            "opaque",
        ),
        ("opencode-v2", json!({"type":"system","text":text}), "0"),
        (
            "pi",
            json!({"type":"message","message":{"role":"system","content":text}}),
            "0",
        ),
    ] {
        let facts = ProjectionFacts::default().with_host_frame(&fact);
        let row = display::project_with_facts(profile, &body, None, &facts).unwrap();
        assert_frame_parts(&row, key, words);
        assert_eq!(row.host_frame.as_ref(), Some(&fact));
        assert_eq!(
            row,
            display::project_with_facts(profile, &body, None, &facts).unwrap()
        );
    }
}

#[test]
fn host_frame_mismatch_keeps_native_text_role_and_parts() {
    let stored = wrapped("original 🚀");
    let fact = HostFrame::verified(0, CODE, &stored).unwrap();
    let head_changed = format!("X{}", &stored[1..]);
    let tail_changed = format!("{}X", &stored[..stored.len() - 1]);
    assert_eq!(stored.len(), head_changed.len());
    assert_eq!(stored.len(), tail_changed.len());
    for (text, candidate) in [
        (head_changed.as_str(), fact.clone()),
        (tail_changed.as_str(), fact.clone()),
        (&stored, HostFrame::new(0, CODE, 53, 232)),
        (&stored, HostFrame::new(0, CODE, 52, 231)),
        (&stored, HostFrame::new(0, "future_frame", 52, 232)),
        (CAPTURE_HEAD, fact.clone()),
    ] {
        assert!(!attribution::verify_host_frame(&candidate, text));
        let body = json!({"role":"system","content":text});
        let row = display::project_with_facts(
            ANTHROPIC,
            &body,
            None,
            &ProjectionFacts::default().with_host_frame(&candidate),
        )
        .unwrap();
        assert_eq!(row, display::project(ANTHROPIC, &body, None).unwrap());
        assert_eq!(row.role, "system");
        assert_eq!(row.text, text);
        assert!(row.host_frame.is_none());
    }
    for (body, candidate) in [
        (
            json!({"role":"system","content":stored}),
            HostFrame::new(1, CODE, 52, 232),
        ),
        (
            json!({"role":"system","content":[{"type":"thinking","text":stored}]}),
            fact,
        ),
    ] {
        let row = display::project_with_facts(
            ANTHROPIC,
            &body,
            None,
            &ProjectionFacts::default().with_host_frame(&candidate),
        )
        .unwrap();
        assert_eq!(row, display::project(ANTHROPIC, &body, None).unwrap());
    }
    let text = wrapped("");
    assert!(HostFrame::verified(0, CODE, &text).is_some());
}

#[test]
fn host_frame_code_wire_shape_is_open_but_validated() {
    for code in ["_1", &"a".repeat(64)] {
        let fact: HostFrame =
            serde_json::from_value(json!({"part":0,"code":code,"head_bytes":52,"tail_bytes":232}))
                .unwrap();
        assert!(!attribution::verify_host_frame(&fact, &wrapped("hello")));
    }
    for code in ["", "upperCase", "bad-code", "é", &"a".repeat(65)] {
        assert!(serde_json::from_value::<HostFrame>(
            json!({"part":0,"code":code,"head_bytes":52,"tail_bytes":232})
        )
        .is_err());
    }
}

#[test]
fn host_frame_does_not_split_text_grouped_into_a_tool_result() {
    let stored = wrapped("not a steer");
    let body = json!({"type":"message","message":{
        "role":"toolResult","toolCallId":"call-1","content":[{"type":"text","text":stored}]
    }});
    let host = HostFrame::verified(0, CODE, &stored).unwrap();
    let row = display::project_with_facts(
        "pi",
        &body,
        None,
        &ProjectionFacts::default().with_host_frame(&host),
    )
    .unwrap();
    assert_eq!(row, display::project("pi", &body, None).unwrap());
    assert_eq!(row.role, "toolResult");
    assert!(row.host_frame.is_none());
    assert!(matches!(&row.parts[0], DisplayPart::ToolResult(result) if result.content == stored));
}

#[test]
fn host_frame_and_attribution_together_are_refused() {
    let stored = wrapped("words");
    let body = json!({"role":"system","content":stored});
    let host = HostFrame::verified(0, CODE, &stored).unwrap();
    let relay = attribution::verified(0, "chat-app", &attribution::header("chat-app")).unwrap();
    let facts = ProjectionFacts::default()
        .with_host_frame(&host)
        .with_attribution(&relay);
    let expected = "attribution and host_frame are mutually exclusive";
    assert_eq!(facts.check(), Err(expected));
    assert_eq!(
        display::project_with_facts(ANTHROPIC, &body, None, &facts)
            .unwrap_err()
            .to_string(),
        expected
    );

    let read = ReadMessage::new(1, "m1", body.clone())
        .with_written_at(5)
        .with_host_frame(host.clone());
    let mut wire_read = json!(read);
    wire_read["attribution"] = json!(relay);
    assert!(ReadMessage::deserialize(wire_read.clone())
        .unwrap_err()
        .to_string()
        .contains(expected));
    assert!(serde_json::from_value::<HistoryEntry>(wire_read)
        .unwrap_err()
        .to_string()
        .contains(expected));
    let both = read.with_attribution(relay.clone());
    assert!(HistoryEntry::from_read(both).is_none());

    let row = display::project_with_facts(
        ANTHROPIC,
        &body,
        None,
        &ProjectionFacts::default().with_host_frame(&host),
    )
    .unwrap();
    let mut wire_row = json!(row);
    wire_row["attribution"] = json!(relay);
    assert!(serde_json::from_value::<DisplayRow>(wire_row)
        .unwrap_err()
        .to_string()
        .contains(expected));
}

#[test]
fn host_frame_preserves_served_keys_and_avoids_bookkeeping_collisions() {
    let word = "hello";
    let stored = wrapped(word);
    let body = json!({"info":{"role":"system"},"parts":[
        {"type":"step-start","id":"bookkeeping-start"},
        {"type":"text","id":"repeat","text":stored},
        {"type":"step-finish","id":"bookkeeping-finish"},
        {"type":"text","id":"repeat","text":stored},
    ]});
    let host = HostFrame::verified(1, CODE, &stored).unwrap();
    let keys: Vec<_> = [
        "words#host_frame:head",
        "words",
        "words#host_frame:tail",
        "untouched",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let facts = ProjectionFacts::default()
        .with_host_frame(&host)
        .with_part_keys(&keys);
    let row = display::project_with_facts("opencode-aisdk", &body, None, &facts).unwrap();
    assert_eq!(
        row,
        display::project_with_facts("opencode-aisdk", &body, None, &facts).unwrap()
    );
    assert_eq!(row.text, word);
    let [head, words, tail, rest] = row.parts.as_slice() else {
        panic!(
            "expected a split frame and one unaffected text part: {:?}",
            row.parts
        );
    };
    assert_eq!(words.part(), "words");
    assert_eq!(
        rest,
        &DisplayPart::Text(display::TextPart::new("untouched", &stored))
    );
    for part in [head, tail] {
        assert!(!keys.iter().any(|key| key == part.part()));
    }
    assert_ne!(head.part(), tail.part());
    let expected = [
        keys[0].as_str(),
        head.part(),
        "words",
        tail.part(),
        keys[2].as_str(),
        "untouched",
    ]
    .map(str::to_owned);
    let raw = HistoryEntry::new(1, "m1", 5, body).with_part_keys(keys.clone());
    let display = DisplayEntry::from_history(&raw, row);
    assert_eq!(display.part_keys.unwrap(), expected);
    assert_eq!(raw.part_keys.unwrap(), keys);
}

#[test]
fn host_frame_omission_preserves_preexisting_wire_bytes() {
    let body = json!({"role":"system","content":"plain"});
    let read = ReadMessage::new(1, "m1", body.clone()).with_written_at(5);
    assert_eq!(
        serde_json::to_string(&read).unwrap(),
        r#"{"ordinal":1,"mid":"m1","message":{"content":"plain","role":"system"},"written_at":5}"#
    );
    let history = HistoryEntry::from_read(read).unwrap();
    assert_eq!(
        serde_json::to_string(&history).unwrap(),
        r#"{"ordinal":1,"mid":"m1","written_at":5,"message":{"content":"plain","role":"system"}}"#
    );
    let row =
        display::project_with_facts(ANTHROPIC, &body, None, &ProjectionFacts::default()).unwrap();
    assert_eq!(
        serde_json::to_string(&row).unwrap(),
        r#"{"role":"system","text":"plain","parts":[{"type":"text","part":"0","text":"plain"}]}"#
    );
}

fn read_wire_fields() -> Value {
    let digest = "a".repeat(64);
    json!({
        "digest": digest, "part_digests": [{"part":"k","digest":digest}], "part_keys": ["k"],
        "ordinal": 1, "mid": "m1", "message": {"role":"system","content":wrapped("words")},
        "run": {"run_id":"r1","episode":"e1","final":true},
        "originals": [{"field":"content","value":"before","provider":"chat-app"}],
        "tool_calls": [{"tool_call_id":"call-1","indeterminate":true}],
        "role": "user", "written_at": 10, "delivered_as": {"mode":"steer"},
        "sender": "chat-app", "send_id": "send-1", "speaks_for": {"agent_id":"helper"},
        "notice": true, "keep": true,
        "host_frame": {"part":0,"code":CODE,"head_bytes":52,"tail_bytes":232}
    })
}

fn verify_wire_round_trip<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(
    wire: Value,
) {
    let mut with_future = wire.clone();
    with_future["future_fact"] = json!({"version":2});
    let entry: T = serde_json::from_value(with_future).unwrap();
    let encoded = serde_json::to_vec(&entry).unwrap();
    // Compare with the input, not just a second decode that could also drop a field.
    assert_eq!(serde_json::from_slice::<Value>(&encoded).unwrap(), wire);
    assert_eq!(serde_json::from_slice::<T>(&encoded).unwrap(), entry);
}

fn verify_both_frame_wire_forms<T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug>(
    mut wire: Value,
) {
    verify_wire_round_trip::<T>(wire.clone());
    wire.as_object_mut().unwrap().remove("host_frame");
    wire["attribution"] = json!({"part":0,"header_bytes":78,"module":"chat-app"});
    verify_wire_round_trip::<T>(wire);
}

#[test]
fn host_frame_read_message_wire_round_trips_all_fields() {
    let mut wire = read_wire_fields();
    wire.as_object_mut().unwrap().extend(
        json!({
            "rich_ui": {"dialect":"test/v1","catalog_digest":"b".repeat(64)},
            "client_message_id": "client-1", "created_at": 8, "completed_at": 9,
            "reasoning_summaries": [{"part":0,"text":"summary"}],
            "notices": [{"part":0,"call_key":"c","send_id":"send-2","sender":"chat-app","text":"notice","delivered_as":"tool_result"}],
            "tool_times": [{"tool_call_id":"call-1","started_at":7}],
            "host_parts": [{"part":"k","host_code":"host_note"}],
            "model_fallback": {"requested":"model-1","answered":"model-2"},
            "runner_entry": {"code":"host_stored_no_reply"}
        }).as_object().unwrap().clone(),
    );
    verify_both_frame_wire_forms::<ReadMessage>(wire);
}

#[test]
fn host_frame_history_entry_wire_round_trips_all_fields() {
    verify_both_frame_wire_forms::<HistoryEntry>(read_wire_fields());
}

#[test]
fn host_frame_display_row_wire_round_trips_all_fields() {
    let wire = json!({
        "host_frame": {"part":0,"code":CODE,"head_bytes":52,"tail_bytes":232},
        "notices": [{"part":0,"call_key":"c","send_id":"send-2","sender":"chat-app","text":"notice","delivered_as":"tool_result"}],
        "output_refs": {"set_id":"set-1","mid":"m1","dialect":"test/v1","count":1,"digest":"a".repeat(64)},
        "role": "user", "text": "words", "parts": [{"type":"text","part":"k","text":"words"}],
        "model": "model-2", "model_fallback": {"requested":"model-1","answered":"model-2"},
        "runner_entry": {"code":"host_stored_no_reply"}, "client_message_id": "client-1",
        "sender": "chat-app", "speaks_for": {"agent_id":"helper"}, "notice": true, "keep": true,
        "rich_ui": {"dialect":"test/v1","catalog_digest":"b".repeat(64)}
    });
    verify_both_frame_wire_forms::<DisplayRow>(wire);
}
