use std::{collections::BTreeMap, fs, path::Path};

use cortexkit_role_llm_runner::{
    send_state_msg_id, vectors::send_state as vectors, SendStateNotice, SEND_STATE_EVENT,
    SEND_STATE_VERSION,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const SESSION: &str = "78854827f6b9ee06bcfec3c818e40ae1";

#[test]
fn send_state_vectors_pin_exact_jcs_bodies_and_envelopes() {
    assert_eq!(SEND_STATE_EVENT, "send_state");
    assert_eq!(SEND_STATE_VERSION, 1);
    let cases = [
        ("sender-a", "sender-a-notice-1", 41, "pending"),
        ("sender-a", "sender-a-notice-1", 44, "active"),
        ("sender-a", "sender-a-notice-2", 45, "superseded"),
        ("sender-a", "sender-a-notice-3", 46, "expired"),
        ("sender-b", "app-send-7", 50, "pending"),
        ("sender-b", "app-send-7", 51, "active"),
        ("sender-b", "app-send-7", 60, "paused"),
        ("sender-b", "app-send-7", 72, "finished"),
    ];
    assert_eq!(vectors::FILES.len(), cases.len() + 1);
    for ((name, contents), (sender, send_id, seq, state)) in vectors::FILES.iter().zip(cases) {
        let envelope: Value = serde_json::from_str(contents).unwrap();
        let notice = SendStateNotice::new(send_id, seq, SESSION, state);
        notice.validate().unwrap();
        let body = envelope["body"].as_str().unwrap().as_bytes();
        assert_eq!(serde_json::to_vec(&notice).unwrap(), body, "{name}");
        assert_eq!(serde_jcs::to_vec(&notice).unwrap(), body, "{name}");
        assert_eq!(
            serde_json::from_slice::<SendStateNotice>(body).unwrap(),
            notice
        );
        assert_eq!(
            envelope,
            json!({
                "body": std::str::from_utf8(body).unwrap(),
                "headers": {
                    "kind": SEND_STATE_EVENT, "send_id": send_id,
                    "seq": seq.to_string(), "session_ref": SESSION
                },
                "nats_msg_id": send_state_msg_id(SESSION, send_id, seq),
                "subject": format!("ck.box_test.event.runner-a.{SEND_STATE_EVENT}.v{SEND_STATE_VERSION}.{sender}")
            }),
            "{name}"
        );
    }
}

#[test]
fn send_state_vectors_match_disk_and_sha256sums() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(vectors::DIR);
    assert_eq!(
        fs::read_dir(&directory).unwrap().count(),
        vectors::FILES.len() + 1
    );
    assert_eq!(
        fs::read_to_string(directory.join("SHA256SUMS")).unwrap(),
        vectors::SHA256SUMS
    );
    let checksums: BTreeMap<_, _> = vectors::SHA256SUMS
        .lines()
        .map(|line| line.split_once("  ").unwrap())
        .map(|(digest, name)| (name, digest))
        .collect();
    assert_eq!(checksums.len(), 9);
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
fn send_state_readers_ignore_unknown_members_and_keep_state_open() {
    let notice: SendStateNotice = serde_json::from_value(json!({
        "send_id": "send:1", "seq": 0, "session_ref": SESSION,
        "state": "future_state", "v": 1, "future_member": {"nested": [1, 2]}
    }))
    .unwrap();
    assert_eq!(
        notice,
        SendStateNotice::new("send:1", 0, SESSION, "future_state")
    );
    notice.validate().unwrap();
    assert_eq!(
        serde_json::to_string(&notice).unwrap(),
        format!(
            r#"{{"send_id":"send:1","seq":0,"session_ref":"{SESSION}","state":"future_state","v":1}}"#
        )
    );
    for state in [
        "pending",
        "active",
        "paused",
        "finished",
        "superseded",
        "expired",
        "retracted",
        "not_delivered",
        " ",
        "future:é",
    ] {
        SendStateNotice::new("send:1", 1, SESSION, state)
            .validate()
            .unwrap();
    }
}

#[test]
fn send_state_validate_reuses_identifier_bounds_and_checks_version_and_state() {
    for send_id in [" ".to_owned(), "a".repeat(257), "send:\"\\\n\0é".to_owned()] {
        let notice = SendStateNotice::new(send_id, 1, SESSION, "pending");
        notice.validate().unwrap();
        let bytes = serde_json::to_vec(&notice).unwrap();
        assert_eq!(bytes, serde_jcs::to_vec(&notice).unwrap());
        assert_eq!(
            serde_json::from_slice::<SendStateNotice>(&bytes).unwrap(),
            notice
        );
    }
    assert!(SendStateNotice::new("", 1, SESSION, "pending")
        .validate()
        .unwrap_err()
        .to_string()
        .contains("send_id"));
    for reference in [
        "".to_owned(),
        "a".repeat(31),
        "a".repeat(33),
        "A".repeat(32),
        "g".repeat(32),
        "é".repeat(16),
    ] {
        assert!(SendStateNotice::new("send:1", 1, reference, "pending")
            .validate()
            .unwrap_err()
            .to_string()
            .contains("session_ref"));
    }
    SendStateNotice::new("send:1", 1, "0123456789abcdef0123456789abcdef", "pending")
        .validate()
        .unwrap();
    let mut notice = SendStateNotice::new("send:1", 1, SESSION, "pending");
    for version in [0, 2, u64::MAX] {
        notice.v = version;
        assert!(notice.validate().unwrap_err().to_string().contains("v:"));
    }
    assert!(SendStateNotice::new("send:1", 1, SESSION, "")
        .validate()
        .unwrap_err()
        .to_string()
        .contains("state"));
}

#[test]
fn send_state_sequence_is_exactly_representable_in_json() {
    let mut notice = SendStateNotice::new("send:1", (1_u64 << 53) - 1, SESSION, "pending");
    notice.validate().unwrap();
    assert_eq!(
        serde_json::to_vec(&notice).unwrap(),
        serde_jcs::to_vec(&notice).unwrap()
    );
    for seq in [1_u64 << 53, u64::MAX] {
        notice.seq = seq;
        assert!(notice.validate().unwrap_err().to_string().contains("seq"));
    }
}

#[test]
fn send_state_body_cap_counts_serialized_bytes_and_never_truncates() {
    let mut notice = SendStateNotice::new("s", 1, SESSION, "x");
    let overhead = serde_json::to_vec(&notice).unwrap().len() - 1;
    notice.state = "x".repeat(512 - overhead);
    assert_eq!(serde_json::to_vec(&notice).unwrap().len(), 512);
    notice.validate().unwrap();
    notice.state.push('x');
    assert_eq!(serde_json::to_vec(&notice).unwrap().len(), 513);
    let before = notice.clone();
    assert!(notice.validate().unwrap_err().to_string().contains("512"));
    assert_eq!(notice, before);
    for state in ["é".repeat(512 - overhead), "\n".repeat(512 - overhead)] {
        notice.state = state;
        assert!(notice.validate().unwrap_err().to_string().contains("512"));
    }
    notice.state = "x".into();
    notice.send_id = "a".repeat(512);
    assert!(notice.validate().unwrap_err().to_string().contains("512"));
}

#[test]
fn send_state_requires_typed_members_and_rejects_duplicates() {
    let valid =
        json!({"send_id":"send:1", "seq":1, "session_ref":SESSION, "state":"pending", "v":1});
    for field in ["send_id", "seq", "session_ref", "state", "v"] {
        let mut missing = valid.clone();
        missing.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<SendStateNotice>(missing).is_err(),
            "{field}"
        );
        for wrong in [Value::Null, json!(true), json!([]), json!({})] {
            let mut malformed = valid.clone();
            malformed[field] = wrong;
            assert!(
                serde_json::from_value::<SendStateNotice>(malformed).is_err(),
                "{field}"
            );
        }
        let duplicate = format!(
            "{},{}:{}}}",
            serde_json::to_string(&valid).unwrap().trim_end_matches('}'),
            serde_json::to_string(field).unwrap(),
            valid[field]
        );
        assert!(
            serde_json::from_str::<SendStateNotice>(&duplicate).is_err(),
            "{field}"
        );
    }
    for field in ["seq", "v"] {
        for wrong in [json!("1"), json!(-1), json!(1.0), json!(1.5)] {
            let mut malformed = valid.clone();
            malformed[field] = wrong;
            assert!(
                serde_json::from_value::<SendStateNotice>(malformed).is_err(),
                "{field}"
            );
        }
    }
    for field in ["send_id", "session_ref", "state"] {
        let mut malformed = valid.clone();
        malformed[field] = json!(1);
        assert!(
            serde_json::from_value::<SendStateNotice>(malformed).is_err(),
            "{field}"
        );
    }
    assert!(serde_json::from_value::<SendStateNotice>(Value::Null).is_err());
}

#[test]
fn send_state_msg_ids_separate_sessions_sends_and_changes() {
    assert_eq!(
        send_state_msg_id(SESSION, "send:1", 41),
        format!("{SESSION}:send:1:41")
    );
    let id = send_state_msg_id(SESSION, "send:1", 41);
    assert_eq!(id, send_state_msg_id(SESSION, "send:1", 41));
    assert_ne!(
        id,
        send_state_msg_id("0123456789abcdef0123456789abcdef", "send:1", 41)
    );
    assert_ne!(id, send_state_msg_id(SESSION, "send:2", 41));
    assert_ne!(id, send_state_msg_id(SESSION, "send:1", 42));
}
