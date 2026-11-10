use cortexkit_usage_steps::{NotClosed, StepRow, StepSource, Verdict};
use serde_json::{json, Value};

const PI_ROW: &str = include_str!("../test-vectors/usage-steps-v1/read/row-message-id.json");

#[test]
fn message_id_round_trips_host_identifiers_separate_from_step_refs() {
    let pi: StepRow = serde_json::from_str(PI_ROW).unwrap();
    assert_eq!(serde_json::to_string(&pi).unwrap(), PI_ROW);
    let raw: Value = serde_json::from_str(PI_ROW).unwrap();
    assert_eq!(raw["message_id"], "entry_ex_1");
    assert_ne!(raw["step_ref"], raw["message_id"]);
    assert_eq!(StepRow::check_closed(&raw), Ok(()));

    let mut opencode = raw;
    opencode["harness"] = json!("opencode");
    opencode["step_ref"] = json!("msg_ex_0001");
    opencode["message_id"] = json!("msg_ex_0001");
    let decoded: StepRow = serde_json::from_value(opencode.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), opencode);
    assert_eq!(StepRow::check_closed(&opencode), Ok(()));
}

#[test]
fn message_id_legacy_rows_and_constructor_keep_absent_member_absent() {
    for legacy in [
        include_str!("../test-vectors/usage-steps-v1/read/row-required-only.json"),
        include_str!("../test-vectors/usage-steps-v1/read/row-every-member.json"),
    ] {
        let decoded: StepRow = serde_json::from_str(legacy).unwrap();
        assert_eq!(serde_json::to_string(&decoded).unwrap(), legacy);
        assert!(serde_json::to_value(decoded)
            .unwrap()
            .get("message_id")
            .is_none());
    }

    let row = StepRow::new(
        "c1",
        "pi",
        StepSource::File,
        "opaque_step_ex",
        "session_ex",
        1,
        10,
        Verdict::Unknown,
        false,
    );
    let encoded = serde_json::to_value(row).unwrap();
    assert!(encoded.get("message_id").is_none());
    let mut null = encoded.clone();
    null["message_id"] = Value::Null;
    let decoded: StepRow = serde_json::from_value(null).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), encoded);
}

#[test]
fn message_id_does_not_change_row_identity() {
    let row: StepRow = serde_json::from_str(PI_ROW).unwrap();
    let mut raw: Value = serde_json::from_str(PI_ROW).unwrap();
    raw["message_id"] = json!("another_entry_ex");
    let updated: StepRow = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(updated.identity(), row.identity());

    raw["message_id"] = json!("entry_ex_1");
    raw["session"] = json!("another_session_ex");
    raw["step_ref"] = json!("another_opaque_step_ex");
    let distinct: StepRow = serde_json::from_value(raw).unwrap();
    assert_ne!(distinct.identity(), row.identity());
}

#[test]
fn message_id_keeps_unknown_member_decoding_tolerant_and_checker_named() {
    let expected: Value = serde_json::from_str(PI_ROW).unwrap();
    let mut raw = expected.clone();
    raw["future_member"] = json!(true);
    assert_eq!(
        StepRow::check_closed(&raw),
        Err(NotClosed::UnknownMember {
            object: "row",
            member: "future_member".to_owned(),
        })
    );
    let decoded: StepRow = serde_json::from_value(raw).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
}

#[test]
fn message_id_uses_the_existing_identifier_string_decoding_rules() {
    let base: Value = serde_json::from_str(PI_ROW).unwrap();
    for invalid in [json!(123), json!(true), json!([]), json!({})] {
        let mut raw = base.clone();
        raw["message_id"] = invalid;
        assert!(serde_json::from_value::<StepRow>(raw).is_err());
    }

    // Readers keep identifier strings verbatim, just as they do for step_ref.
    for identifier in [String::new(), "x".repeat(4096), "entry\n\u{0}".to_owned()] {
        let mut raw = base.clone();
        raw["message_id"] = json!(identifier);
        let decoded: StepRow = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
    }
}
