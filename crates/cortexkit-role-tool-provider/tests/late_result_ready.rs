use std::{collections::BTreeMap, fs, path::Path};

use cortexkit_role_tool_provider::{
    late_result_ready_msg_id, late_result_ready_reader, vectors::late_result_ready as vectors,
    LateResultReady, LATE_RESULT_READY_EVENT, LATE_RESULT_READY_VERSION,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn fixture(name: &str) -> &'static str {
    vectors::FILES
        .iter()
        .find_map(|(file, contents)| (*file == name).then_some(*contents))
        .unwrap_or_else(|| panic!("unknown late-result-ready fixture: {name}"))
}

#[test]
fn late_result_ready_vectors_pin_subjects_ids_and_exact_payload_bytes() {
    assert_eq!(LATE_RESULT_READY_EVENT, "late_result_ready");
    assert_eq!(LATE_RESULT_READY_VERSION, 1);
    for (case, call_key, outcome, event_id) in [
        ("ok", "call:ok", "ok", "settlement:1"),
        ("error", "call:error", "tool_denied", "settlement:2"),
        ("unknown", "call:unknown", "unknown", "settlement:3"),
    ] {
        let ready = LateResultReady::new(call_key, outcome);
        ready.validate().unwrap();
        let payload = fixture(&format!("{case}/payload.json")).as_bytes();
        assert_eq!(serde_json::to_vec(&ready).unwrap(), payload, "{case}");
        assert_eq!(
            serde_json::from_slice::<LateResultReady>(payload).unwrap(),
            ready
        );

        let reader = late_result_ready_reader("reserved:reader").unwrap();
        assert_eq!(reader, "reader");
        assert_eq!(
            fixture(&format!("{case}/subject.txt")),
            format!(
                "event.provider.{LATE_RESULT_READY_EVENT}.v{LATE_RESULT_READY_VERSION}.{reader}\n"
            )
        );
        assert_eq!(
            fixture(&format!("{case}/nats-msg-id.txt")),
            format!("{}\n", late_result_ready_msg_id(event_id, reader))
        );
    }
}

#[test]
fn late_result_ready_vectors_match_disk_and_sha256sums() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(vectors::DIR);
    assert_eq!(
        fs::read_to_string(directory.join("SHA256SUMS")).unwrap(),
        vectors::SHA256SUMS
    );
    let checksums: BTreeMap<_, _> = vectors::SHA256SUMS
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            (name, digest)
        })
        .collect();
    assert_eq!(checksums.len(), 10);
    assert_eq!(checksums.len(), vectors::SHA256SUMS.lines().count());
    assert_eq!(checksums.len(), vectors::FILES.len());
    let names: Vec<_> = vectors::SHA256SUMS
        .lines()
        .map(|line| line.split_once("  ").unwrap().1)
        .collect();
    assert_eq!(
        names,
        vectors::FILES
            .iter()
            .map(|(name, _)| *name)
            .collect::<Vec<_>>()
    );
    for (name, contents) in vectors::FILES {
        assert_eq!(
            fs::read(directory.join(name)).unwrap(),
            contents.as_bytes(),
            "{name}"
        );
        assert_eq!(
            format!("{:x}", Sha256::digest(contents.as_bytes())),
            checksums[name],
            "{name}"
        );
    }
}

#[test]
fn late_result_ready_readers_ignore_unknown_members_and_keep_open_outcomes() {
    let ready: LateResultReady = serde_json::from_value(json!({
        "call_key": "call:1",
        "outcome": "future_error_code",
        "future_member": {"nested": [1, 2]}
    }))
    .unwrap();
    ready.validate().unwrap();
    assert_eq!(ready, LateResultReady::new("call:1", "future_error_code"));
    assert_eq!(
        serde_json::to_vec(&ready).unwrap(),
        br#"{"call_key":"call:1","outcome":"future_error_code"}"#
    );
    for outcome in ["ok", "unknown", "tool_denied", " ", "future:é"] {
        LateResultReady::new("call:1", outcome).validate().unwrap();
    }
}

#[test]
fn late_result_ready_validate_reuses_call_key_bounds_and_requires_outcome() {
    for key in ["!".to_owned(), "~".repeat(256), "call:\"\\".to_owned()] {
        let ready = LateResultReady::new(key, "ok");
        ready.validate().unwrap();
        let encoded = serde_json::to_vec(&ready).unwrap();
        assert_eq!(
            serde_json::from_slice::<LateResultReady>(&encoded).unwrap(),
            ready
        );
    }
    for key in [
        "".to_owned(),
        "a".repeat(257),
        "has space".to_owned(),
        "tab\tkey".to_owned(),
        "line\nkey".to_owned(),
        "nul\0key".to_owned(),
        "del\u{7f}".to_owned(),
        "non-ascii:é".to_owned(),
    ] {
        assert!(LateResultReady::new(key, "ok")
            .validate()
            .unwrap_err()
            .to_string()
            .contains("call_key"));
    }
    assert!(LateResultReady::new("call:1", "")
        .validate()
        .unwrap_err()
        .to_string()
        .contains("outcome"));
}

#[test]
fn late_result_ready_requires_both_string_members_and_refuses_duplicates() {
    for value in [
        json!({}),
        json!({"call_key": "call:1"}),
        json!({"outcome": "ok"}),
        json!({"call_key": null, "outcome": "ok"}),
        json!({"call_key": 1, "outcome": "ok"}),
        json!({"call_key": ["call:1"], "outcome": "ok"}),
        json!({"call_key": "call:1", "outcome": null}),
        json!({"call_key": "call:1", "outcome": 1}),
        json!({"call_key": "call:1", "outcome": {"code": "ok"}}),
        Value::Null,
    ] {
        assert!(serde_json::from_value::<LateResultReady>(value).is_err());
    }
    for duplicate in [
        r#"{"call_key":"call:1","call_key":"call:2","outcome":"ok"}"#,
        r#"{"call_key":"call:1","outcome":"ok","outcome":"unknown"}"#,
    ] {
        assert!(serde_json::from_str::<LateResultReady>(duplicate).is_err());
    }
}

#[test]
fn late_result_ready_reader_requires_a_valid_reserved_module_id() {
    for reader in [
        "reader".to_owned(),
        "0".to_owned(),
        "reader_1-test".to_owned(),
        "a".repeat(63),
    ] {
        assert_eq!(
            late_result_ready_reader(&format!("reserved:{reader}")),
            Some(reader.as_str())
        );
    }
    for principal in [
        "",
        "reader",
        "direct",
        "reserved",
        "reserved:",
        "reserved:Reader",
        "reserved:_reader",
        "reserved:-reader",
        "reserved:reader.extra",
        "reserved:*",
        "reserved:>",
        "reserved:reader.*",
        "reserved:reader/other",
        "reserved:reader:other",
        "reserved:reader ",
        "reserved:réader",
        "reserved:reader\n",
        "reserved:reserved:reader",
    ] {
        assert_eq!(late_result_ready_reader(principal), None, "{principal:?}");
    }
    assert_eq!(
        late_result_ready_reader(&format!("reserved:{}", "a".repeat(64))),
        None
    );
}

#[test]
fn late_result_ready_msg_ids_preserve_stable_settlement_and_reader_identity() {
    assert_eq!(
        late_result_ready_msg_id("event:1", "reader"),
        "event:1:reader"
    );
    assert_eq!(
        late_result_ready_msg_id("event:1", "reader"),
        late_result_ready_msg_id("event:1", "reader")
    );
    assert_ne!(
        late_result_ready_msg_id("event:1", "reader"),
        late_result_ready_msg_id("event:2", "reader")
    );
    assert_ne!(
        late_result_ready_msg_id("event:1", "reader"),
        late_result_ready_msg_id("event:1", "reader-2")
    );
}
