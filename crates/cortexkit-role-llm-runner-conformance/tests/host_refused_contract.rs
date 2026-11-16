use cortexkit_role_llm_runner::{
    errors,
    run::RunResult,
    send::{self, SendReply, SendState},
    vectors,
};
use serde_json::{json, Value};

#[test]
fn host_refused_send_and_status_samples_decode_and_round_trip() {
    assert_eq!(send::REASON_HOST_REFUSED, "host_refused");
    for name in ["send.json", "send-status.json"] {
        let file: Value = serde_json::from_str(vectors::file(name).unwrap()).unwrap();
        let samples: Vec<_> = file["replies"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|case| case["reply"]["reason"] == "host_refused")
            .collect();
        assert_eq!(samples.len(), 1, "{name}");
        let wire = &samples[0]["reply"];
        assert_eq!(
            wire,
            &json!({
                "state": "not_delivered", "run_id": "run-16",
                "submission_id": "sub-5", "reason": "host_refused"
            })
        );
        let reply: SendReply = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(reply.send_state(), SendState::NotDelivered);
        assert_eq!(reply.reason.as_deref(), Some(send::REASON_HOST_REFUSED));
        assert!(reply.delivered.is_none());
        assert_eq!(serde_json::to_value(reply).unwrap(), *wire);
    }
}

#[test]
fn host_refused_run_error_samples_decode_and_round_trip() {
    assert_eq!(errors::run_error_classes::HOST_REFUSED, "host_refused");
    for (named, wire) in [
        (
            errors::causes::ATTACHMENT_UNREADABLE,
            "attachment_unreadable",
        ),
        (
            errors::causes::HOST_CONFIGURATION_INVALID,
            "host_configuration_invalid",
        ),
        (
            errors::causes::HOST_VERSION_UNSUPPORTED,
            "host_version_unsupported",
        ),
        (errors::causes::HOST_POLICY_REFUSED, "host_policy_refused"),
    ] {
        assert_eq!(named, wire);
    }
    assert_eq!(
        errors::provider_codes::HOST_POLICY_REFUSED,
        "host_policy_refused"
    );
    assert!(errors::provider_codes::CODES.contains(&errors::provider_codes::HOST_POLICY_REFUSED));
    assert!(!errors::is_retryable(
        errors::provider_codes::HOST_POLICY_REFUSED
    ));
    let file: Value = serde_json::from_str(vectors::file("run-result.json").unwrap()).unwrap();
    let samples: Vec<_> = file["answers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|case| case["answer"]["error"]["class"] == "host_refused")
        .collect();
    assert_eq!(samples.len(), 4);
    for (cause, code, detail) in [
        ("attachment_unreadable", "provider_error", None),
        ("host_configuration_invalid", "provider_error", None),
        ("host_version_unsupported", "provider_error", None),
        (
            "host_policy_refused",
            "host_policy_refused",
            Some("attachments_disabled"),
        ),
    ] {
        let wire = &samples
            .iter()
            .find(|case| case["answer"]["error"]["cause"] == cause)
            .unwrap()["answer"];
        let result: RunResult = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(result.state, "error");
        assert!(result.run_state().is_terminal());
        let error = result.error.as_ref().unwrap();
        assert_eq!(error.class, errors::run_error_classes::HOST_REFUSED);
        assert_eq!(error.cause.as_deref(), Some(cause));
        assert_eq!(error.provider_code, code);
        assert_eq!(error.provider_detail_code.as_deref(), detail);
        assert_eq!(serde_json::to_value(result).unwrap(), *wire);
    }
}

#[test]
fn host_unavailable_status_and_run_error_samples_decode_and_round_trip() {
    assert_eq!(send::REASON_HOST_UNAVAILABLE, "host_unavailable");
    assert_eq!(
        errors::run_error_classes::HOST_UNAVAILABLE,
        "host_unavailable"
    );
    let status: Value = serde_json::from_str(vectors::file("send-status.json").unwrap()).unwrap();
    let wire = &status["replies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["reply"]["reason"] == "host_unavailable")
        .unwrap()["reply"];
    assert_eq!(
        wire,
        &json!({
            "state": "not_delivered", "run_id": "run-8",
            "submission_id": "sub-4", "reason": "host_unavailable"
        })
    );
    let reply: SendReply = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(reply.send_state(), SendState::NotDelivered);
    assert_eq!(reply.reason.as_deref(), Some(send::REASON_HOST_UNAVAILABLE));
    assert!(reply.delivered.is_none());
    assert_eq!(serde_json::to_value(reply).unwrap(), *wire);

    let results: Value = serde_json::from_str(vectors::file("run-result.json").unwrap()).unwrap();
    let wire = &results["answers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["answer"]["error"]["class"] == "host_unavailable")
        .unwrap()["answer"];
    let result: RunResult = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(result.state, "error");
    assert_eq!(result.run_id, "run-8");
    assert!(result.run_state().is_terminal());
    let error = result.error.as_ref().unwrap();
    assert_eq!(error.class, errors::run_error_classes::HOST_UNAVAILABLE);
    assert_eq!(error.provider_code, errors::provider_codes::PROVIDER_ERROR);
    assert_eq!(error.cause.as_deref(), Some("host_unreachable"));
    assert_eq!(serde_json::to_value(result).unwrap(), *wire);
}
