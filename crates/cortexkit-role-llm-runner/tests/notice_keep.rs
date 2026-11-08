use cortexkit_role_llm_runner::{
    describe::{check_describe, DescribeProblem},
    display::DisplayRow,
    history::HistoryEntry,
    read::ReadMessage,
    send::SendRequest,
};
use serde_json::{json, Value};

#[test]
fn notice_keep_send_vectors_check_admission_and_normalization() {
    let vectors: Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/send-notice-keep.json"
    ))
    .unwrap();
    for case in vectors.as_array().unwrap() {
        let request: SendRequest = serde_json::from_value(case["request"].clone()).unwrap();
        let result = request.check_notice_keep(
            case["declared"].as_bool().unwrap(),
            case["scope_owner"].as_str(),
            case["attested_sender"].as_str().unwrap(),
        );
        if let Some(field) = case["field"].as_str() {
            assert_eq!(result, Err(field), "{}", case["name"]);
        } else {
            assert_eq!(
                result,
                Ok(case["keep"].as_bool().unwrap()),
                "{}",
                case["name"]
            );
            let encoded = serde_json::to_value(request).unwrap();
            assert_eq!(encoded.get("keep"), case["normalized"].get("keep"));
        }
    }
}

#[test]
fn notice_keep_rejects_wrong_types_and_legacy_rides() {
    for keep in [Value::Null, json!(1), json!("true"), json!({}), json!([])] {
        let request: SendRequest = serde_json::from_value(json!({
            "prompt":"words", "send_id":"send-a", "notice":true, "keep":keep
        }))
        .unwrap();
        assert_eq!(
            request.check_notice_keep(true, Some("owner"), "owner"),
            Err("keep")
        );
    }
    let error = serde_json::from_value::<SendRequest>(json!({
        "prompt":"words", "send_id":"send-a", "notice":true, "keep":true,
        "delivery":"tool_result"
    }))
    .unwrap_err();
    assert!(error.to_string().contains("delivery"));
}

#[test]
fn notice_keep_false_is_absent_and_true_survives_entry_surfaces() {
    let plain = SendRequest::new("words", "send-a");
    assert_eq!(
        serde_json::to_value(plain.clone().with_keep(false)).unwrap(),
        serde_json::to_value(plain).unwrap()
    );
    let marked = SendRequest::new("words", "send-a")
        .with_notice(true)
        .with_keep(true);
    assert_eq!(
        marked.check_notice_keep(true, Some("owner"), "owner"),
        Ok(true)
    );
    for keep in [false, true] {
        let mut read = ReadMessage::new(1, "mid-a", json!({"role":"user", "content":"words"}))
            .with_written_at(10)
            .with_notice(true);
        read.keep = keep;
        let history = HistoryEntry::from_read(read.clone()).unwrap();
        let mut row = DisplayRow::new("user", vec![]).with_notice(true);
        row.keep = keep;
        for surface in [
            serde_json::to_value(&read).unwrap(),
            serde_json::to_value(history).unwrap(),
            serde_json::to_value(row).unwrap(),
        ] {
            assert_eq!(
                surface.get("keep"),
                if keep { Some(&Value::Bool(true)) } else { None }
            );
        }
        assert_eq!(
            serde_json::from_value::<ReadMessage>(serde_json::to_value(&read).unwrap()).unwrap(),
            read
        );
    }
}

#[test]
fn notice_keep_describe_requires_message_marks_by_name() {
    let value = json!({
        "majors":[{"version":"llm-runner/v1", "stability":"alpha", "ops":cortexkit_role_llm_runner::REQUIRED_OPS}],
        "implementation_version":"test",
        "session_capabilities_from":"baseline",
        "capabilities":["notice_keep"]
    });
    let errors = check_describe(&value).unwrap_err();
    assert!(
        errors.contains(&DescribeProblem::Undecodable(
            "notice_keep_without_message_marks".into()
        )),
        "{errors:?}"
    );
}
