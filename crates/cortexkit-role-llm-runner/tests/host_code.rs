use cortexkit_role_llm_runner::{
    display::{
        self, host_codes, DisplayPart, DisplayRow, FilePart, OtherPart, TextPart, ToolCallPart,
        ToolResultPart,
    },
    read::{HostPart, ReadMessage, TagReadAnswer},
};
use serde_json::{json, Value};

fn part() -> Value {
    json!({"type":"text", "part":"0", "text":"[Request interrupted by user]\n", "host":true})
}

fn answer() -> Value {
    let file: Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/display-host-parts.json"
    ))
    .unwrap();
    file["tag_read_answers"][0]["answer"].clone()
}

#[test]
fn host_code_decode_round_trip_preserves_open_vocabulary() {
    for code in ["interrupted", "interrupted_for_tool_use", "future_notice_7"] {
        let mut part = part();
        part["host_code"] = json!(code);
        let decoded: DisplayPart = serde_json::from_value(part.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), part);
        let mut answer = answer();
        answer["host_code"] = json!(code);
        let decoded: TagReadAnswer = serde_json::from_value(answer.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), answer);
    }
}

#[test]
fn host_code_decode_refuses_missing_or_false_host() {
    for host in [None, Some(json!(false))] {
        let mut part = part();
        let mut answer = answer();
        for raw in [&mut part, &mut answer] {
            raw["host_code"] = json!("interrupted");
            raw.as_object_mut().unwrap().remove("host");
            if let Some(host) = &host {
                raw["host"] = host.clone();
            }
        }
        assert!(serde_json::from_value::<DisplayPart>(part).is_err());
        assert!(serde_json::from_value::<TagReadAnswer>(answer).is_err());
    }
}

#[test]
fn host_code_decode_validates_shape_not_vocabulary() {
    for invalid in [
        json!(""),
        json!("a".repeat(65)),
        json!("Upper"),
        json!("has-dash"),
        json!("with space"),
        json!("é"),
        json!(null),
        json!(true),
        json!(1),
    ] {
        let mut part = part();
        part["host_code"] = invalid.clone();
        assert!(
            serde_json::from_value::<DisplayPart>(part).is_err(),
            "{invalid}"
        );
        let mut answer = answer();
        answer["host_code"] = invalid.clone();
        assert!(
            serde_json::from_value::<TagReadAnswer>(answer).is_err(),
            "{invalid}"
        );
    }
    for valid in ["0".to_owned(), "_".into(), "a".repeat(64)] {
        let mut part = part();
        part["host_code"] = json!(valid);
        let decoded: DisplayPart = serde_json::from_value(part.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), part);
        let mut answer = answer();
        answer["host_code"] = json!(valid);
        let decoded: TagReadAnswer = serde_json::from_value(answer.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), answer);
    }
}

#[test]
fn host_parts_read_fact_decode_round_trip() {
    let file: Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/display-host-codes.json"
    ))
    .unwrap();
    for case in file["cases"].as_array().unwrap() {
        let entry: ReadMessage = serde_json::from_value(case["read"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&entry).unwrap(), case["read"]);
        let row: DisplayRow = serde_json::from_value(case["row"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&row).unwrap(), case["row"]);
        let mut projected = display::project("anthropic-messages", &entry.message, None).unwrap();
        assert!(projected
            .parts
            .iter()
            .all(|part| !part.host() && part.host_code().is_none()));
        let keys: Vec<&str> = projected.parts.iter().map(DisplayPart::part).collect();
        entry.check_host_parts(&projected.role, &keys).unwrap();
        entry.check_model_fallback(&projected.role).unwrap();
        row.check_model_fallback().unwrap();
        assert!(projected.model_fallback.is_none());
        projected.model_fallback = entry.model_fallback.clone();
        for part in &mut projected.parts {
            if let Some(fact) = entry
                .host_parts
                .iter()
                .find(|fact| fact.part == part.part())
            {
                *part = match &fact.host_code {
                    Some(code) => part.clone().with_host_code(code),
                    None => part.clone().with_host(true),
                };
            }
        }
        assert_eq!(projected, row);
        assert_eq!(
            serde_json::to_value(&entry.message).unwrap(),
            case["read"]["message"]
        );
    }
    for raw in file["invalid_parts"].as_array().unwrap() {
        assert!(serde_json::from_value::<DisplayPart>(raw.clone()).is_err());
    }
    for invalid in file["invalid_facts"].as_array().unwrap() {
        let entry = ReadMessage::new(0, "invalid", json!({}))
            .with_role(invalid["role"].as_str().unwrap())
            .with_host_parts(serde_json::from_value(invalid["host_parts"].clone()).unwrap());
        assert!(
            entry
                .check_host_parts(entry.role.as_deref().unwrap(), &["0"])
                .is_err(),
            "{}",
            invalid["name"]
        );
    }
}

#[test]
fn host_code_builders_set_both_members_and_clearing_host_clears_code() {
    let parts = [
        DisplayPart::Text(TextPart::new("0", "text")),
        DisplayPart::Reasoning(TextPart::new("1", "reasoning")),
        DisplayPart::ToolCall(ToolCallPart::new("2", "call", "echo", &json!({}), None)),
        DisplayPart::ToolResult(ToolResultPart::new(
            "3",
            "call",
            false,
            "result",
            vec![],
            None,
        )),
        DisplayPart::File(FilePart::url("4", "https://example.com/file", None)),
        DisplayPart::Other(OtherPart::new("5", "future")),
    ];
    for part in parts {
        let unmarked = serde_json::to_value(&part).unwrap();
        assert!(unmarked.get("host_code").is_none());
        let coded = part.with_host_code("future_7");
        assert!(coded.host());
        assert_eq!(coded.host_code(), Some("future_7"));
        let raw = serde_json::to_value(&coded).unwrap();
        assert_eq!(
            serde_json::from_value::<DisplayPart>(raw.clone()).unwrap(),
            coded
        );
        for field in ["host", "host_code"] {
            let mut invalid = raw.clone();
            invalid.as_object_mut().unwrap().remove(field);
            if field == "host" {
                assert!(serde_json::from_value::<DisplayPart>(invalid).is_err());
            } else {
                assert!(serde_json::from_value::<DisplayPart>(invalid)
                    .unwrap()
                    .host_code()
                    .is_none());
            }
        }
        assert_eq!(
            serde_json::to_value(coded.with_host(false)).unwrap(),
            unmarked
        );
    }
    let entry = ReadMessage::new(1, "m", json!({"role":"user"}));
    let answer = TagReadAnswer::new(entry, 0);
    let unmarked = serde_json::to_value(&answer).unwrap();
    assert!(unmarked.get("host_code").is_none());
    let coded = answer.with_host_code(host_codes::INTERRUPTED);
    assert!(coded.host);
    assert_eq!(coded.host_code.as_deref(), Some("interrupted"));
    assert_eq!(
        serde_json::from_value::<TagReadAnswer>(serde_json::to_value(&coded).unwrap()).unwrap(),
        coded
    );
    assert_eq!(
        serde_json::to_value(coded.with_host(false)).unwrap(),
        unmarked
    );
    assert_eq!(
        host_codes::INTERRUPTED_TEXT,
        "[Request interrupted by user]\n"
    );
    assert_eq!(
        host_codes::INTERRUPTED_FOR_TOOL_USE,
        "interrupted_for_tool_use"
    );
    assert_eq!(
        host_codes::INTERRUPTED_FOR_TOOL_USE_TEXT,
        "[Request interrupted by user for tool use]\n"
    );
    assert_eq!(host_codes::SERVER_FALLBACK, "server_fallback");
    assert_eq!(
        host_codes::SERVER_FALLBACK_PREFIX,
        "cortexkit-server-fallback-v1:"
    );
    assert_eq!(
        host_codes::SERVER_FALLBACK_USAGE_PREFIX,
        "cortexkit-server-fallback-usage-v1:"
    );
}

#[test]
fn host_code_unknown_part_and_older_readers_keep_text_and_host() {
    let unknown: DisplayPart = serde_json::from_value(json!({
        "type":"future_part", "part":"key", "host":true, "host_code":"future_code"
    }))
    .unwrap();
    assert!(unknown.host());
    assert_eq!(unknown.host_code(), Some("future_code"));
    assert_eq!(
        serde_json::to_value(&unknown).unwrap()["host_code"],
        "future_code"
    );
    for invalid in [json!(""), json!(null), json!("Upper")] {
        assert!(serde_json::from_value::<DisplayPart>(json!({
            "type":"future_part", "part":"key", "host":true, "host_code":invalid
        }))
        .is_err());
    }

    // These are the required wire fields before host_code was added, including
    // the host mark.
    // Serde's unknown-member tolerance is what lets older readers decode them.
    #[derive(serde::Deserialize)]
    struct OlderPart {
        part: String,
        text: String,
        #[serde(default)]
        host: bool,
    }
    #[derive(serde::Deserialize)]
    struct OlderRead {
        ordinal: u64,
        mid: String,
        message: Value,
    }
    #[derive(serde::Deserialize)]
    struct OlderAnswer {
        entry: OlderRead,
        part: u32,
        #[serde(default)]
        host: bool,
    }
    let mut raw = part();
    raw["host_code"] = json!("interrupted");
    let old: OlderPart = serde_json::from_value(raw).unwrap();
    assert_eq!(old.part, "0");
    assert_eq!(old.text, "[Request interrupted by user]\n");
    assert!(old.host);
    let raw = json!({"ordinal":1,"mid":"m","message":{"role":"user"},
        "host_parts":[{"part":"key","host_code":"future_code"}]});
    let old: OlderRead = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(old.ordinal, 1);
    assert_eq!(old.mid, "m");
    assert_eq!(old.message, json!({"role":"user"}));
    let old: OlderAnswer = serde_json::from_value(json!({"entry":raw,"part":0,
        "host":true,"host_code":"future_code"}))
    .unwrap();
    assert_eq!(old.entry.mid, "m");
    assert_eq!(old.part, 0);
    assert!(old.host);
}

#[test]
fn host_parts_validate_key_order_role_and_shape_without_parsing_opaque_body() {
    let body = json!({"native":"opaque"});
    let entry = ReadMessage::new(1, "m", body.clone())
        .with_role("user")
        .with_host_parts(vec![
            HostPart::new("part-b").with_host_code("future_7"),
            HostPart::new("part-z"),
        ]);
    assert_eq!(entry.message, body);
    entry
        .check_host_parts("user", &["part-a", "part-b", "part-z"])
        .unwrap();
    assert!(entry
        .check_host_parts("tool", &["part-b", "part-z"])
        .is_err());
    assert!(entry
        .check_host_parts("user", &["part-z", "part-b"])
        .is_err());
    assert!(entry.check_host_parts("user", &["part-b"]).is_err());
    assert!(entry
        .clone()
        .with_role("system")
        .check_host_parts("user", &["part-b", "part-z"])
        .is_err());
    let empty = entry.with_host_parts(vec![]);
    assert!(serde_json::to_value(&empty)
        .unwrap()
        .get("host_parts")
        .is_none());
    let mut raw = serde_json::to_value(&empty).unwrap();
    raw["host_parts"] = json!([]);
    assert_eq!(serde_json::from_value::<ReadMessage>(raw).unwrap(), empty);
    for invalid in [json!(""), json!("Bad"), json!(null), json!("a".repeat(65))] {
        assert!(
            serde_json::from_value::<HostPart>(json!({"part":"key","host_code":invalid})).is_err()
        );
    }
    assert!(host_codes::check(false, Some("interrupted")).is_err());
    assert!(host_codes::check(true, Some("Bad")).is_err());
}

#[test]
fn host_parts_assistant_role_validates_and_other_roles_are_refused() {
    let entry = ReadMessage::new(1, "reply", json!({"native":"opaque"}))
        .with_role("assistant")
        .with_host_parts(vec![HostPart::new("0")]);
    entry.check_host_parts("assistant", &["0"]).unwrap();
    for fact in [
        HostPart::new("0"),
        HostPart::new("0").with_host_code("future_code"),
    ] {
        let entry = entry.clone().with_host_parts(vec![fact]);
        for role in ["system", "tool", "developer", "skill", "future_role"] {
            assert!(entry.check_host_parts(role, &["0"]).is_err(), "{role}");
            assert!(
                entry
                    .clone()
                    .with_role(role)
                    .check_host_parts("assistant", &["0"])
                    .is_err(),
                "{role}"
            );
        }
    }
    entry
        .with_host_parts(vec![])
        .check_host_parts("system", &["0"])
        .unwrap();
}

#[test]
fn host_parts_synthetic_accepts_optional_codes_and_validates_stored_keys() {
    for code in [
        None,
        Some("continued_after_interruption"),
        Some("future_code"),
    ] {
        let mut fact = HostPart::new("part-b");
        if let Some(code) = code {
            fact = fact.with_host_code(code);
        }
        let entry = ReadMessage::new(1, "native-synthetic", json!({"type":"synthetic"}))
            .with_role("synthetic")
            .with_host_parts(vec![fact.clone(), HostPart::new("part-z")]);
        entry
            .check_host_parts("synthetic", &["part-a", "part-b", "part-z"])
            .unwrap();
        entry
            .clone()
            .with_role("user")
            .check_host_parts("synthetic", &["part-b", "part-z"])
            .unwrap();
        entry
            .check_host_parts("user", &["part-b", "part-z"])
            .unwrap();
        assert!(entry.check_host_parts("synthetic", &["part-b"]).is_err());
        assert!(entry
            .check_host_parts("synthetic", &["part-z", "part-b"])
            .is_err());
        assert!(entry
            .clone()
            .with_host_parts(vec![fact.clone(), fact])
            .check_host_parts("synthetic", &["part-b", "part-z"])
            .is_err());
        let raw = serde_json::to_value(&entry).unwrap();
        assert_eq!(serde_json::from_value::<ReadMessage>(raw).unwrap(), entry);
    }
}

#[test]
fn host_code_synthetic_samples_keep_read_display_and_tag_marks_equal() {
    let file: Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/display-synthetic-host-codes.json"
    ))
    .unwrap();
    for case in file["cases"].as_array().unwrap() {
        let coded: ReadMessage = serde_json::from_value(case["read"].clone()).unwrap();
        let row: DisplayRow = serde_json::from_value(case["row"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&coded).unwrap(), case["read"]);
        assert_eq!(serde_json::to_value(&row).unwrap(), case["row"]);
        assert_eq!(
            row.parts[0].host_code(),
            Some(host_codes::CONTINUED_AFTER_INTERRUPTION)
        );
        assert_eq!(
            row.text.as_bytes(),
            host_codes::CONTINUED_AFTER_INTERRUPTION_TEXT.as_bytes()
        );
        let projected =
            display::project(file["profile"].as_str().unwrap(), &coded.message, None).unwrap();
        assert_eq!(projected.role, "synthetic");
        assert!(projected.parts.iter().all(|part| !part.host()));
        for mark in [None, Some(false), Some(true)] {
            let mut entry = coded.clone();
            let mut shown = projected.clone();
            if let Some(with_code) = mark {
                entry.host_parts = vec![if with_code {
                    HostPart::new("0").with_host_code(host_codes::CONTINUED_AFTER_INTERRUPTION)
                } else {
                    HostPart::new("0")
                }];
                shown.parts[0] = if with_code {
                    shown.parts[0]
                        .clone()
                        .with_host_code(host_codes::CONTINUED_AFTER_INTERRUPTION)
                } else {
                    shown.parts[0].clone().with_host(true)
                };
            } else {
                entry.host_parts.clear();
            }
            let keys: Vec<&str> = shown.parts.iter().map(DisplayPart::part).collect();
            entry.check_host_parts(&shown.role, &keys).unwrap();
            let marked: Vec<HostPart> = shown
                .parts
                .iter()
                .filter(|part| part.host())
                .map(|part| {
                    let fact = HostPart::new(part.part());
                    match part.host_code() {
                        Some(code) => fact.with_host_code(code),
                        None => fact,
                    }
                })
                .collect();
            assert_eq!(entry.host_parts, marked);
            if mark == Some(true) {
                assert_eq!(shown, row);
            }
            assert_eq!(shown.text, row.text);
            assert_eq!(entry.message, coded.message);
        }
    }
    for case in file["tag_read_answers"].as_array().unwrap() {
        let answer: TagReadAnswer = serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&answer).unwrap(), case["answer"]);
        assert_eq!(answer.entry.role.as_deref(), Some("synthetic"));
        assert_eq!(
            answer.host_code.as_deref(),
            Some(host_codes::CONTINUED_AFTER_INTERRUPTION)
        );
        assert_eq!(
            TagReadAnswer::new(answer.entry.clone(), answer.part)
                .with_host_code(host_codes::CONTINUED_AFTER_INTERRUPTION),
            answer
        );
    }
    for case in file["invalid_cases"].as_array().unwrap() {
        let entry: ReadMessage = serde_json::from_value(case["read"].clone()).unwrap();
        assert!(entry.check_host_parts("system", &["0"]).is_err());
    }
}

#[test]
fn host_code_opencode_continuation_constants_pin_exact_bytes() {
    for (code, expected_code, text, expected_text) in [
        (host_codes::CONTINUED_AFTER_INTERRUPTION, "continued_after_interruption",
         host_codes::CONTINUED_AFTER_INTERRUPTION_TEXT,
         b"The previous response was interrupted. Continue from where you left off without repeating completed content.".as_slice()),
        (host_codes::CONTINUED_AFTER_OUTPUT_LIMIT, "continued_after_output_limit",
         host_codes::CONTINUED_AFTER_OUTPUT_LIMIT_TEXT,
         b"Your last response hit the output token limit (stop reason: length). Do not apologize, recap, or repeat yourself. Break the remaining work into smaller pieces.".as_slice()),
        (host_codes::CONTINUED_AFTER_RESTART, "continued_after_restart",
         host_codes::CONTINUED_AFTER_RESTART_TEXT,
         b"The server restarted while you were working. Continue from where you left off without repeating completed work.".as_slice()),
    ] {
        assert_eq!(code, expected_code);
        assert_eq!(text.as_bytes(), expected_text);
        host_codes::check(true, Some(code)).unwrap();
    }
}

#[test]
fn host_parts_assistant_reasoning_and_tag_answer_round_trip() {
    let raw = json!({"role":"assistant", "text":"", "parts":[{
        "type":"reasoning", "part":"0", "text":"\u{2060}",
        "host":true, "host_code":"server_fallback"
    }]});
    let row: DisplayRow = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(row).unwrap(), raw);
    let body = json!({"role":"assistant", "content":[{
        "type":"thinking", "thinking":"\u{2060}",
        "signature":"cortexkit-server-fallback-v1:requested|answered"
    }]});
    let entry = ReadMessage::new(1, "reply", body.clone())
        .with_role("assistant")
        .with_host_parts(vec![
            HostPart::new("0").with_host_code(host_codes::SERVER_FALLBACK)
        ]);
    entry.check_host_parts("assistant", &["0"]).unwrap();
    let answer = TagReadAnswer::new(entry, 0).with_host_code(host_codes::SERVER_FALLBACK);
    let raw = serde_json::to_value(&answer).unwrap();
    assert_eq!(raw["host"], true);
    assert_eq!(raw["host_code"], "server_fallback");
    assert_eq!(raw["entry"]["message"], body);
    assert_eq!(
        serde_json::from_value::<TagReadAnswer>(raw).unwrap(),
        answer
    );
}
