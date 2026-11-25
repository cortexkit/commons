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

const LIST_VECTORS: &str = "test-vectors/llm-runner-v1/steer-frame-claude-code-v2/";

fn list_vector(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(LIST_VECTORS)
        .join(name);
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn host_frame_followed_tail_requires_its_exact_byte_count_and_newline() {
    use attribution::host_frames::CLAUDE_CODE_MID_TURN_TAIL_FOLLOWED as FOLLOWED;

    let words = "Checking both locks 🗝";
    let last = wrapped(words);
    let followed = format!("{last}\n");
    assert_eq!(FOLLOWED.len(), 233);
    assert_eq!(FOLLOWED, format!("{CAPTURE_TAIL}\n"));
    let frame = HostFrame::verified(1, CODE, &followed).expect("recognizes the 233-byte tail");
    assert_eq!(frame.tail_bytes, 233);
    assert!(attribution::verify_host_frame(&frame, &followed));
    assert!(!attribution::verify_host_frame(&frame, &last));
    assert!(!attribution::verify_host_frame(
        &frame,
        &format!("{followed}\n")
    ));
    let last_frame = HostFrame::new(1, CODE, 52, 232);
    assert!(attribution::verify_host_frame(&last_frame, &last));
    assert!(!attribution::verify_host_frame(&last_frame, &followed));
}

#[test]
fn host_frames_read_decoder_accepts_two_frames() {
    let wire = list_vector("two-frames-read.json");
    let read: ReadMessage = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(
        read.host_frames()
            .iter()
            .map(|frame| frame.part)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(json!(read), wire);
    let history = HistoryEntry::from_read(read).unwrap();
    assert_eq!(json!(history), wire);
    let decoded: HistoryEntry = serde_json::from_value(wire).unwrap();
    assert_eq!(decoded.host_frames(), history.host_frames());
}

#[test]
fn host_frames_samples_project_the_exact_rows_and_serve_only_verified_items() {
    for (name, input) in [
        ("two-frames", "two-frames-read.json"),
        ("one-frame", "one-frame-read.json"),
        ("bad-tail", "bad-tail-candidates.json"),
    ] {
        let read: ReadMessage = serde_json::from_value(list_vector(input)).unwrap();
        let row = display::project_with_facts(
            ANTHROPIC,
            &read.message,
            None,
            &ProjectionFacts::default().with_host_frames(read.host_frames()),
        )
        .unwrap();
        let projected = list_vector(&format!("{name}-display.json"));
        assert_eq!(json!(row), projected, "{name}");
        let decoded: DisplayRow = serde_json::from_value(projected).unwrap();
        assert_eq!(decoded, row);
        let plain = display::project(ANTHROPIC, &read.message, None).unwrap();
        assert_eq!(row.parts[0], plain.parts[0], "the tool result is unchanged");
        assert!(!row.notice);
        assert!(row.notices.is_empty());
        if name == "bad-tail" {
            assert_eq!(row.parts.last(), plain.parts.last());
        }

        let served = read
            .clone()
            .with_host_frames(ANTHROPIC, read.host_frames())
            .unwrap();
        assert_eq!(json!(served), list_vector(&format!("{name}-read.json")));
        assert_eq!(row.host_frames(), served.host_frames());
    }
}

#[test]
fn host_frames_wire_and_projection_refuse_bad_list_shapes() {
    let wire = list_vector("two-frames-read.json");
    let row = list_vector("two-frames-display.json");
    let first = wire["host_frames"][0].clone();
    let second = wire["host_frames"][1].clone();
    let duplicate = list_vector("refused-read.json");
    assert!(serde_json::from_value::<ReadMessage>(duplicate.clone()).is_err());
    let relay = json!({"part":0,"module":"chat","header_bytes":10});
    let mut bad_code = first.clone();
    bad_code["code"] = json!("a-invalid");
    for (frames, other) in [
        (json!([]), None),
        (Value::Null, None),
        (duplicate["host_frames"].clone(), None),
        (json!([second, first]), None),
        (json!([bad_code]), None),
        (wire["host_frames"].clone(), Some(("host_frame", first))),
        (
            wire["host_frames"].clone(),
            Some(("host_frame", Value::Null)),
        ),
        (wire["host_frames"].clone(), Some(("attribution", relay))),
    ] {
        let invalid = |mut base: Value| {
            base["host_frames"] = frames.clone();
            if let Some((key, value)) = &other {
                base[*key] = value.clone();
            }
            base
        };
        assert!(serde_json::from_value::<ReadMessage>(invalid(wire.clone())).is_err());
        assert!(serde_json::from_value::<HistoryEntry>(invalid(wire.clone())).is_err());
        assert!(serde_json::from_value::<DisplayRow>(invalid(row.clone())).is_err());
    }
    let read: ReadMessage = serde_json::from_value(wire).unwrap();
    let [first, second] = read.host_frames() else {
        panic!("two frame facts")
    };
    for frames in [
        vec![],
        vec![first.clone(), first.clone()],
        vec![second.clone(), first.clone()],
    ] {
        let facts = ProjectionFacts::default().with_host_frames(&frames);
        assert!(facts.check().is_err());
        assert!(display::project_with_facts(ANTHROPIC, &read.message, None, &facts).is_err());
    }
    let facts = ProjectionFacts::default()
        .with_host_frame(first)
        .with_host_frames(read.host_frames());
    assert!(facts.check().is_err());
}

#[test]
fn host_frames_skip_all_unverified_parts_without_serving_an_empty_list() {
    let read: ReadMessage = serde_json::from_value(list_vector("two-frames-read.json")).unwrap();
    let mut unknown = read.host_frames()[0].clone();
    unknown.code = "unrecognized_code".into();
    let non_text = HostFrame::new(0, CODE, 52, 232);
    let non_existent = HostFrame::new(30, CODE, 52, 232);
    let invalid = [non_text.clone(), unknown.clone(), non_existent.clone()];
    let served = read.clone().with_host_frames(ANTHROPIC, &invalid).unwrap();
    assert!(served.host_frames().is_empty());
    assert!(json!(served).get("host_frames").is_none());
    let facts = ProjectionFacts::default().with_host_frames(&invalid);
    let plain = display::project(ANTHROPIC, &read.message, None).unwrap();
    assert_eq!(
        display::project_with_facts(ANTHROPIC, &read.message, None, &facts).unwrap(),
        plain
    );
    let valid = read.host_frames()[1].clone();
    let mixed = [non_text, unknown, valid.clone(), non_existent];
    let row = display::project_with_facts(
        ANTHROPIC,
        &read.message,
        None,
        &ProjectionFacts::default().with_host_frames(&mixed),
    )
    .unwrap();
    assert_eq!(row.host_frames(), &[valid]);
    assert_eq!(row.parts[..2], plain.parts[..2]);
}

#[test]
fn host_frames_single_list_preserves_legacy_projection_and_normalizes_legacy_wire() {
    let read: ReadMessage = serde_json::from_value(list_vector("one-frame-read.json")).unwrap();
    let frame = read.host_frames()[0].clone();
    let mut legacy_wire = json!(read);
    legacy_wire.as_object_mut().unwrap().remove("host_frames");
    legacy_wire["host_frame"] = json!(frame);
    let legacy_read: ReadMessage = serde_json::from_value(legacy_wire).unwrap();
    assert_eq!(legacy_read.host_frames(), read.host_frames());
    assert_eq!(
        HistoryEntry::from_read(legacy_read).unwrap().host_frames(),
        read.host_frames()
    );

    let stored = read.message["content"][1]["text"].clone();
    let part_zero = [HostFrame::new(0, CODE, 52, 232)];
    for (profile, body, frames) in [
        (ANTHROPIC, read.message.clone(), read.host_frames()),
        (
            "opencode-aisdk",
            json!({"info":{"role":"system"},"parts":[{"id":"opaque","type":"text","text":stored}]}),
            part_zero.as_slice(),
        ),
        (
            "opencode-v2",
            json!({"type":"system","text":stored}),
            part_zero.as_slice(),
        ),
        (
            "pi",
            json!({"type":"message","message":{"role":"system","content":stored}}),
            part_zero.as_slice(),
        ),
    ] {
        let project = |facts| display::project_with_facts(profile, &body, None, &facts).unwrap();
        let legacy = project(ProjectionFacts::default().with_host_frame(&frames[0]));
        let row = project(ProjectionFacts::default().with_host_frames(frames));
        assert_eq!(legacy.host_frames(), frames);
        assert_eq!(row.host_frames(), frames);
        let mut legacy_json = json!(legacy);
        legacy_json.as_object_mut().unwrap().remove("host_frame");
        let mut row_json = json!(row);
        row_json.as_object_mut().unwrap().remove("host_frames");
        assert_eq!(
            serde_json::to_vec(&legacy_json).unwrap(),
            serde_json::to_vec(&row_json).unwrap(),
            "{profile}"
        );
    }
}

#[test]
fn host_frames_publish_all_host_keys_in_stored_order() {
    let read: ReadMessage = serde_json::from_value(list_vector("two-frames-read.json")).unwrap();
    let content = &read.message["content"];
    let body = json!({"info":{"role":"system"},"parts":[
        {"type":"step-start"}, content[1], content[2], {"type":"step-finish"},
    ]});
    let keys = [
        "first#host_frame:head",
        "first",
        "second",
        "second#host_frame:tail",
    ]
    .map(str::to_owned);
    let facts = ProjectionFacts::default()
        .with_part_keys(&keys)
        .with_host_frames(read.host_frames());
    let row = display::project_with_facts("opencode-aisdk", &body, None, &facts).unwrap();
    assert_eq!(row.role, "user");
    assert_eq!(
        row,
        display::project_with_facts("opencode-aisdk", &body, None, &facts).unwrap()
    );
    let history = HistoryEntry::new(1, "m1", 5, body).with_part_keys(keys.to_vec());
    let display = DisplayEntry::from_history(&history, row);
    assert_eq!(
        display.part_keys.unwrap(),
        [
            "first#host_frame:head",
            "first#host_frame:head#",
            "first",
            "first#host_frame:tail",
            "second#host_frame:head",
            "second",
            "second#host_frame:tail#",
            "second#host_frame:tail",
        ]
    );
    assert_eq!(history.part_keys.unwrap(), keys);
}
