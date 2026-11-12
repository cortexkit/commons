use cortexkit_role_llm_runner::{
    live::{LiveEvent, RunEnded},
    read::{HeadMeta, LastRunState},
    run::{RunReason, RunResult, RunStatus},
    subscribe::SubscribeEvent,
};
use serde_json::{json, Value};

#[cfg(feature = "test-vectors")]
#[test]
fn account_limit_status_vector_pins_pause_and_resume_floor() {
    let bytes = cortexkit_role_llm_runner::vectors::file("run_status.account_limit.json").unwrap();
    let status: RunStatus = serde_json::from_str(bytes).unwrap();
    assert_eq!(status.state, "paused");
    assert_eq!(status.reason, Some(RunReason::AccountLimit));
    assert_eq!(status.resume_not_before_ms, Some(1_800_000_060_250));
    let error = status.error.as_ref().unwrap();
    assert_eq!(error.provider_code, "account_limit_exhausted");
    assert_eq!(error.retry_after_secs, Some(60.0));
    assert_eq!(error.resets_at_ms, Some(1_800_000_060_000));
    assert_eq!(
        format!("{}\n", serde_json::to_string(&status).unwrap()),
        bytes
    );
}

#[test]
fn account_limit_status_retains_resume_floor() {
    let body = json!({
        "state": "paused",
        "reason": "account_limit",
        "resume_not_before_ms": 1_800_000_060_250_u64,
        "error": {
            "class": "transient",
            "message": "Account limit reached",
            "provider_code": "account_limit_exhausted",
            "retry_after_secs": 60.0,
            "resets_at_ms": 1_800_000_060_000_u64
        }
    });
    let status: RunStatus = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(status.reason, Some(RunReason::AccountLimit));
    assert_eq!(status.resume_not_before_ms, Some(1_800_000_060_250));
    assert_eq!(serde_json::to_value(status).unwrap(), body);
    let built = RunStatus::new("paused")
        .with_reason(RunReason::AccountLimit)
        .with_resume_not_before_ms(1_800_000_060_250);
    assert_eq!(built.resume_not_before_ms, Some(1_800_000_060_250));
}

#[test]
fn account_limit_head_retains_resume_floor() {
    let body = json!({
        "run_id": "run-1",
        "state": "paused",
        "reason": "account_limit",
        "resume_not_before_ms": 1_800_000_060_250_u64
    });
    let state: LastRunState = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(state.resume_not_before_ms, Some(1_800_000_060_250));
    assert_eq!(serde_json::to_value(&state).unwrap(), body);
    let head = HeadMeta::new(1, "log-1", 0, 1_800_000_000_000).with_last_run_state(state);
    let encoded = serde_json::to_value(&head).unwrap();
    assert_eq!(encoded["last_run_state"], body);
    assert_eq!(serde_json::from_value::<HeadMeta>(encoded).unwrap(), head);
}

#[test]
fn account_limit_paused_control_event_retains_resume_floor() {
    let body = json!({
        "kind": "control",
        "cursor": {"wal_seq": 7},
        "event": "paused",
        "run_id": "run-1",
        "reason": "account_limit",
        "resume_not_before_ms": 1_800_000_060_250_u64
    });
    let event: SubscribeEvent = serde_json::from_value(body.clone()).unwrap();
    assert_eq!(serde_json::to_value(event).unwrap(), body);
}

#[test]
fn account_limit_typed_run_paused_event_retains_resume_floor() {
    let event = json!({
        "type": "run_paused",
        "run_id": "run-1",
        "reason": "account_limit",
        "resume_not_before_ms": 1_800_000_060_250_u64
    });
    let frame = json!({"kind": "control", "cursor": {"wal_seq": 7}, "event": event});
    let parsed: SubscribeEvent = serde_json::from_value(frame.clone()).unwrap();
    let typed = parsed.live_event().unwrap();
    assert_eq!(serde_json::to_value(&typed).unwrap(), event);
    assert_eq!(LiveEvent::decode_strict(&event).unwrap(), typed);
    assert_eq!(
        typed,
        LiveEvent::RunPaused(
            RunEnded::new("run-1", "account_limit").with_resume_not_before_ms(1_800_000_060_250)
        )
    );
    assert_eq!(serde_json::to_value(parsed).unwrap(), frame);
}

#[test]
fn older_pause_bodies_decode_without_a_resume_floor() {
    let status = json!({"state": "paused", "reason": "auth_required"});
    let parsed: RunStatus = serde_json::from_value(status.clone()).unwrap();
    assert_eq!(parsed.resume_not_before_ms, None);
    assert_eq!(serde_json::to_value(parsed).unwrap(), status);
    let head = json!({"run_id": "run-1", "state": "paused", "reason": "restart"});
    let parsed: LastRunState = serde_json::from_value(head.clone()).unwrap();
    assert_eq!(parsed.resume_not_before_ms, None);
    assert_eq!(serde_json::to_value(parsed).unwrap(), head);
    let event = json!({"kind": "control", "event": "paused", "reason": "restart"});
    let parsed: SubscribeEvent = serde_json::from_value(event.clone()).unwrap();
    assert_eq!(serde_json::to_value(parsed).unwrap(), event);
    let event = json!({"type": "run_paused", "run_id": "run-1", "reason": "restart"});
    let parsed = LiveEvent::decode_strict(&event).unwrap();
    assert_eq!(
        parsed,
        LiveEvent::RunPaused(RunEnded::new("run-1", "restart"))
    );
    assert_eq!(serde_json::to_value(parsed).unwrap(), event);
}

#[test]
fn account_limit_reason_round_trips_through_its_named_variant() {
    let reason: RunReason = serde_json::from_value(json!("account_limit")).unwrap();
    assert_eq!(reason, RunReason::AccountLimit);
    assert_eq!(
        serde_json::to_value(reason).unwrap(),
        json!("account_limit")
    );
}

#[test]
fn unknown_run_reason_is_preserved() {
    let reason: RunReason = serde_json::from_value(json!("future_pause")).unwrap();
    assert_eq!(reason, RunReason::Unknown("future_pause".into()));
    assert_eq!(serde_json::to_value(reason).unwrap(), json!("future_pause"));
}

#[test]
fn account_limit_result_carries_original_error_without_resume_floor() {
    let body = json!({
        "run_id": "run-1",
        "state": "paused",
        "reason": "account_limit",
        "error": {
            "class": "transient",
            "message": "Account limit reached",
            "provider_code": "account_limit_exhausted",
            "retry_after_secs": 60.0,
            "resets_at_ms": 1_800_000_060_000_u64
        }
    });
    let result: RunResult = serde_json::from_value(body.clone()).unwrap();
    let encoded = serde_json::to_value(result).unwrap();
    assert_eq!(encoded, body);
    assert_eq!(encoded.get("resume_not_before_ms"), None::<&Value>);
}
