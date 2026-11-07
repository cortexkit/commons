use cortexkit_role_tool_display::{
    wake::*, Principal, DISPLAY_TOO_LARGE, DISPLAY_WRITER_UNVERIFIED,
};
use serde_json::{json, Value};

fn fixture() -> Value {
    serde_json::from_str(include_str!("../test-vectors/tool-display-v1/wakes.json")).unwrap()
}

fn writer() -> Principal {
    Principal::Reserved {
        module_id: "wake-writer".into(),
    }
}

#[test]
fn wake_constructors_and_registry_agents_validate() {
    let mut agent = WakeAgent::new("agent_a1b2c3d4");
    agent.display_name = Some("Juniper".into());
    agent.check().unwrap();
    for bad in [
        "agent_A1b2c3d4",
        "module-example",
        "agent_123",
        "agent_zzzzzzzz",
    ] {
        assert!(WakeAgent::new(bad).check().is_err());
    }
    let item = WakeItem::new(
        "notice",
        json!({"family":"module_notice", "title":"Progress"})
            .as_object()
            .unwrap()
            .clone(),
        0,
        "Ready",
    );
    let write = WakeWrite::new(
        Principal::Direct,
        "session-constructor",
        "send-constructor",
        WakeRecord::new(vec![item]),
    );
    write.check(&writer()).unwrap();
}

#[test]
fn wake_vectors_cover_every_kind_caps_and_keyed_riding_notice() {
    let fixture = fixture();
    assert_eq!(WAKE_RECORD_WRITER_CAPABILITY, fixture["tokens"]["writer"]);
    assert_eq!(WAKE_RECORD_DISPLAY_CAPABILITY, fixture["tokens"]["display"]);
    for case in fixture["writes"].as_array().unwrap() {
        let write: WakeWrite = serde_json::from_value(case["write"].clone()).unwrap();
        let bytes = write.check(&writer()).unwrap();
        assert_eq!(serde_json::to_value(&write).unwrap(), case["write"]);
        if case["tag"] == "at_caps" {
            assert_eq!(bytes.len(), 16_384);
            assert_eq!(write.body.items.len(), 64);
            assert_eq!(write.body.omitted_items, Some(5));
        }
        if case["tag"] == "all_kinds" {
            let kinds: std::collections::BTreeSet<_> = write
                .body
                .items
                .iter()
                .map(|item| item.kind.as_str())
                .collect();
            assert_eq!(
                kinds,
                [
                    "peer_message",
                    "room_post",
                    "worker_question",
                    "work_result",
                    "sidekick_answer",
                    "approval_card",
                    "notice"
                ]
                .into_iter()
                .collect()
            );
        }
        if case["tag"] == "keyed_riding_notice" {
            assert_eq!(write.send_id, case["notice"]["send_id"]);
            assert_eq!(case["send"]["send_id"], case["notice"]["send_id"]);
            assert_eq!(case["send"]["delivery"], json!(["tool_result"]));
        }
    }
}

#[test]
fn wake_refusal_vectors_and_claimed_senders_are_rejected() {
    let fixture = fixture();
    for case in fixture["refusals"].as_array().unwrap() {
        let write: WakeWrite = serde_json::from_value(case["write"].clone()).unwrap();
        let principal: Principal = serde_json::from_value(case["writer"].clone()).unwrap();
        let error = write.check(&principal).unwrap_err();
        assert_eq!(error.code, case["code"], "{}", case["tag"]);
    }
    let valid = &fixture["writes"][0]["write"];
    for field in ["envelope", "body"] {
        let mut value = valid.clone();
        if field == "envelope" {
            value["sender"] = json!("other-writer");
        } else {
            value["body"]["sender"] = json!("other-writer");
        }
        assert!(serde_json::from_value::<WakeWrite>(value).is_err());
    }
}

#[test]
fn wake_reader_keeps_unknown_kinds_but_writer_refuses_them() {
    let value = json!({"items":[{"kind":"future_kind", "refs":{"future":[1,2]}, "at":0, "excerpt":"Future wake"}]});
    let record: WakeRecord = serde_json::from_value(value.clone()).unwrap();
    record.check().unwrap();
    assert_eq!(serde_json::to_value(&record).unwrap(), value);
    assert_eq!(
        record.check_writer().unwrap_err().detail.unwrap()["field"],
        "kind"
    );
}

#[test]
fn wake_checks_all_required_refs_and_peer_agents() {
    let fixture = fixture();
    let record: WakeRecord =
        serde_json::from_value(fixture["writes"][0]["write"]["body"].clone()).unwrap();
    for item in &record.items {
        for field in item.refs.keys() {
            let mut missing = item.clone();
            missing.refs.remove(field);
            if item.kind == "work_result" && ["run", "item"].contains(&field.as_str()) {
                missing.check().unwrap();
            } else {
                assert!(missing.check().is_err(), "{} missing {field}", item.kind);
            }
            let mut malformed = item.clone();
            malformed.refs.insert(field.clone(), Value::Null);
            assert!(malformed.check().is_err(), "{} null {field}", item.kind);
        }
    }
    for field in ["from", "to"] {
        let mut item = serde_json::to_value(&record.items[0]).unwrap();
        item.as_object_mut().unwrap().remove(field);
        assert!(serde_json::from_value::<WakeItem>(item)
            .unwrap()
            .check()
            .is_err());
    }
    let mut fold = record.items[1].clone();
    fold.refs.insert("first_seq".into(), json!(20));
    fold.refs.insert("last_seq".into(), json!(19));
    assert!(fold.check().is_err());
    let mut approval = record.items[5].clone();
    approval.refs.insert("id".into(), json!("el_example"));
    approval.check().unwrap();
    for bad in ["acc_", "el_", "other_example"] {
        approval.refs.insert("id".into(), json!(bad));
        assert!(approval.check().is_err());
    }
}

#[test]
fn wake_limits_count_characters_and_canonical_bytes_without_truncating() {
    let mut record: WakeRecord =
        serde_json::from_value(fixture()["writes"][1]["write"]["body"].clone()).unwrap();
    record.check().unwrap();
    record.items.push(record.items[0].clone());
    assert_eq!(record.check().unwrap_err().code, DISPLAY_TOO_LARGE);
    record.items.pop();
    record.items[0].refs.insert("extra".into(), json!("x"));
    assert_eq!(record.check().unwrap_err().code, DISPLAY_TOO_LARGE);
    let mut item = record.items[0].clone();
    item.excerpt = format!("{}…", "é".repeat(199));
    item.check().unwrap();
    item.excerpt.push('x');
    assert_eq!(item.check().unwrap_err().field, "excerpt");
}

#[test]
fn wake_keys_use_attested_writer_session_and_send_not_runner_or_body() {
    let mut write: WakeWrite =
        serde_json::from_value(fixture()["writes"][0]["write"].clone()).unwrap();
    let key = wake_idempotency_key(&writer(), &write).unwrap();
    assert_eq!(
        key,
        br#"["session-example",{"kind":"reserved","module_id":"wake-writer"},"send-example"]"#
    );
    write.runner = Principal::Direct;
    write.body.items.clear();
    assert_eq!(wake_idempotency_key(&writer(), &write).unwrap(), key);
    write.session = "session-moved".into();
    assert_ne!(wake_idempotency_key(&writer(), &write).unwrap(), key);
    write.session = "session-example".into();
    write.send_id = "send-other".into();
    assert_ne!(wake_idempotency_key(&writer(), &write).unwrap(), key);
    write.send_id = "send-example".into();
    let other = Principal::Reserved {
        module_id: "other-writer".into(),
    };
    assert_ne!(wake_idempotency_key(&other, &write).unwrap(), key);
    assert_eq!(
        write.check(&Principal::Direct).unwrap_err().code,
        DISPLAY_WRITER_UNVERIFIED
    );
}

#[test]
fn wake_lookup_vectors_cover_moved_rows_and_ambiguity() {
    for case in fixture()["lookups"].as_array().unwrap() {
        let row = &case["row"];
        let records: Vec<_> = case["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|record| {
                record["sender"] == row["sender"] && record["send_id"] == row["send_id"]
            })
            .collect();
        let same = records
            .iter()
            .find(|record| record["session"] == row["session"]);
        let result = same.copied().or_else(|| {
            if records.len() == 1 {
                Some(records[0])
            } else {
                None
            }
        });
        assert_eq!(
            result
                .map(|record| record["session"].clone())
                .unwrap_or(Value::Null),
            case["matched_session"],
            "{}",
            case["tag"]
        );
    }
}
