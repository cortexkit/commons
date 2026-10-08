use cortexkit_role_compaction_provider::{
    answer::{CompactionMessage, Coverage, Range, RefuseDetail, StepAnswer},
    errors::RefuseCode,
    status::{DescendsFrom, Estimate, StepStatus},
};
use serde_json::{json, Value};

fn host_vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/compaction-provider-v1/host-runner.json"
    ))
    .unwrap()
}

fn status() -> StepStatus {
    StepStatus::new(
        "s",
        "host",
        "r1",
        "l",
        "step",
        "user_turn",
        "m",
        Estimate::new(3),
        1,
    )
}

#[test]
fn step_ancestry_round_trip() {
    for case in host_vectors()["requests"].as_array().unwrap() {
        let status: StepStatus = serde_json::from_value(case["request"].clone()).unwrap();
        assert_eq!(
            status.descends_from,
            Some(DescendsFrom::new("old-lineage", 2))
        );
        assert_eq!(serde_json::to_value(&status).unwrap(), case["request"]);
        let again: StepStatus =
            serde_json::from_slice(&serde_json::to_vec(&status).unwrap()).unwrap();
        assert_eq!(again, status);
    }
    assert_eq!(
        status()
            .with_descends_from(DescendsFrom::new("old", 0))
            .descends_from,
        Some(DescendsFrom::new("old", 0))
    );
}

#[test]
fn summary_coverage_round_trip() {
    for case in host_vectors()["summaries"].as_array().unwrap() {
        let answer: StepAnswer = serde_json::from_value(case["answer"].clone()).unwrap();
        let StepAnswer::CompactionMessage { coverage, .. } = &answer else {
            panic!("not a summary");
        };
        assert_eq!(coverage, &Some(Coverage::new("m2", 2)));
        assert_eq!(serde_json::to_value(&answer).unwrap(), case["answer"]);
        let again: StepAnswer =
            serde_json::from_slice(&serde_json::to_vec(&answer).unwrap()).unwrap();
        assert_eq!(again, answer);
    }
    let answer = StepAnswer::CompactionMessage {
        request_id: "r1".into(),
        compaction: CompactionMessage::new("c", 2, Range::new(0, 1), vec![]),
        coverage: Some(Coverage::new("m0", 0)),
    };
    assert_eq!(
        serde_json::to_value(answer).unwrap()["coverage"],
        json!({"end_mid": "m0", "ordinal": 0})
    );
}

#[test]
fn history_gap_refusals_round_trip_and_ignore_unrelated_codes() {
    for case in host_vectors()["refusals"].as_array().unwrap() {
        let answer: StepAnswer = serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(serde_json::to_value(&answer).unwrap(), case["answer"]);
        assert_eq!(answer.retryable(), Some(true));
        assert_eq!(
            answer.history_gap_from(),
            case["answer"]["detail"]["history_gap_from"].as_u64()
        );
        let again: StepAnswer =
            serde_json::from_slice(&serde_json::to_vec(&answer).unwrap()).unwrap();
        assert_eq!(again, answer);
    }
    for code in [
        RefuseCode::WindowTooSmall,
        RefuseCode::Misconfigured,
        RefuseCode::ProviderBusy,
        RefuseCode::Unknown("unknown".into()),
    ] {
        let retryable = code.retryable();
        let answer = StepAnswer::Refuse {
            request_id: "r1".into(),
            code,
            reason: "not a history refusal".into(),
            provider_code: None,
            detail: Some(RefuseDetail::new().with_history_gap_from(0)),
        };
        assert_eq!(answer.history_gap_from(), None);
        assert_eq!(answer.retryable(), Some(retryable));
    }
    let answer = StepAnswer::Refuse {
        request_id: "r1".into(),
        code: RefuseCode::HistoryUnreadable,
        reason: "missing history".into(),
        provider_code: None,
        detail: Some(RefuseDetail::new().with_history_gap_from(0)),
    };
    assert_eq!(answer.history_gap_from(), Some(0));
}

#[test]
fn legacy_step_bytes_unchanged() {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../test-vectors/compaction-provider-v1/status.json"
    ))
    .unwrap();
    for case in legacy["requests"].as_array().unwrap() {
        let status: StepStatus = serde_json::from_value(case["request"].clone()).unwrap();
        assert!(status.descends_from.is_none());
        assert_eq!(
            serde_json::to_vec(&serde_json::to_value(&status).unwrap()).unwrap(),
            serde_json::to_vec(&case["request"]).unwrap(),
            "{}",
            case["name"]
        );
    }
    assert_eq!(
        serde_json::to_string(&status()).unwrap(),
        r#"{"session":"s","harness":"host","request_id":"r1","lineage_id":"l","step_id":"step","step_kind":"user_turn","model":"m","estimate":{"request_tokens":3},"messages":[],"now":1}"#
    );
}

#[test]
fn absent_answer_fields_preserve_legacy_bytes() {
    let legacy: Value = serde_json::from_str(include_str!(
        "../../../test-vectors/compaction-provider-v1/answers.json"
    ))
    .unwrap();
    for case in legacy["answers"].as_array().unwrap() {
        let answer: StepAnswer = serde_json::from_value(case["answer"].clone()).unwrap();
        match &answer {
            StepAnswer::CompactionMessage { coverage, .. } => assert!(coverage.is_none()),
            StepAnswer::Refuse { detail, .. } => assert!(detail.is_none()),
            _ => {}
        }
        // The expected bytes come from the unchanged vectors, not from a
        // second use of the serializer under test.
        assert_eq!(
            serde_json::to_vec(&serde_json::to_value(&answer).unwrap()).unwrap(),
            serde_json::to_vec(&case["answer"]).unwrap(),
            "{}",
            case["name"]
        );
    }
    assert_eq!(serde_json::to_string(&RefuseDetail::new()).unwrap(), "{}");
    assert_eq!(
        serde_json::to_string(&StepAnswer::CompactionMessage {
            request_id: "r1".into(),
            compaction: CompactionMessage::new("c", 1, Range::new(0, 0), vec![]),
            coverage: None,
        })
        .unwrap(),
        r#"{"answer":"compaction_message","request_id":"r1","compaction":{"compaction_id":"c","version":1,"range":{"from":0,"to":0},"replacement":[]}}"#
    );
}

#[test]
fn new_history_fields_keep_u64_precision_and_lenient_objects() {
    let status = status().with_descends_from(DescendsFrom::new("old", u64::MAX));
    let mut wire = serde_json::to_value(&status).unwrap();
    wire["unknown"] = json!(true);
    wire["descends_from"]["unknown"] = json!(true);
    assert_eq!(serde_json::from_value::<StepStatus>(wire).unwrap(), status);
    let coverage: Coverage =
        serde_json::from_value(json!({"end_mid": "m", "ordinal": u64::MAX, "unknown": true}))
            .unwrap();
    assert_eq!(coverage, Coverage::new("m", u64::MAX));
    let detail: RefuseDetail =
        serde_json::from_value(json!({"history_gap_from": u64::MAX, "unknown": true})).unwrap();
    assert_eq!(detail, RefuseDetail::new().with_history_gap_from(u64::MAX));
    let vectors = host_vectors();
    for case in vectors["undecodable_requests"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<StepStatus>(case["request"].clone()).is_err(),
            "{}",
            case["name"]
        );
    }
    for case in vectors["undecodable_answers"].as_array().unwrap() {
        assert!(
            serde_json::from_value::<StepAnswer>(case["answer"].clone()).is_err(),
            "{}",
            case["name"]
        );
    }
}
