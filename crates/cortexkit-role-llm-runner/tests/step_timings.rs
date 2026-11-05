use cortexkit_role_llm_runner::step_events::{
    decode_step_finished, StepEventDecodeError, StepFinished, StepRef,
};
use serde_json::json;

const SUBJECT: &str = "ck.account.live.runner.step.finished.v1";

fn event() -> StepFinished {
    StepFinished::new(
        StepRef::new("runner", "ref", "log", 1),
        "run",
        "step",
        1.try_into().unwrap(),
        "completed",
        "host",
        "session",
        "provider",
        "model",
        10,
        20,
    )
    .with_attempt_id("attempt")
}

#[test]
fn timings_accept_unknown_zero_equal_and_ordered_measurements() {
    for (sent, first) in [
        (None, None),
        (Some(0), None),
        (None, Some(0)),
        (None, Some(u64::MAX)),
        (Some(0), Some(0)),
        (Some(1), Some(0)),
        (Some(1), Some(1)),
        (Some(u64::MAX), Some(u64::MAX)),
    ] {
        let mut event = event();
        event.sent_to_last_content_ms = sent;
        event.first_to_last_content_ms = first;
        assert_eq!(event.check(), Ok(()), "{sent:?}, {first:?}");
        let bytes = serde_json::to_vec(&event).unwrap();
        assert_eq!(decode_step_finished(SUBJECT, &bytes).unwrap(), event);
    }
}

#[test]
fn producer_check_names_first_to_last_exceeding_sent_to_last() {
    for (sent, first) in [(0, 1), (u64::MAX - 1, u64::MAX)] {
        let event = event()
            .with_sent_to_last_content_ms(sent)
            .with_first_to_last_content_ms(first);
        assert_eq!(event.check(), Err("first_to_last_content_ms"));
    }
}

#[test]
fn decoding_keeps_an_event_whose_timings_break_the_ordering_rule() {
    // A reader must not drop a step's usage over a measurement fault, so
    // decoding applies only the identity rules.
    for (sent, first) in [(0, 1), (u64::MAX - 1, u64::MAX)] {
        let event = event()
            .with_sent_to_last_content_ms(sent)
            .with_first_to_last_content_ms(first);
        let decoded = decode_step_finished(SUBJECT, &serde_json::to_vec(&event).unwrap()).unwrap();
        assert_eq!(decoded, event);
    }
}

#[test]
fn decoding_still_refuses_a_bad_identity() {
    let mut body = serde_json::to_value(event()).unwrap();
    body["attempt_id"] = json!("not a valid attempt id");
    assert!(matches!(
        decode_step_finished(SUBJECT, &serde_json::to_vec(&body).unwrap()),
        Err(StepEventDecodeError::InvalidField("attempt_id"))
    ));
}

#[test]
fn timings_and_decision_are_independently_optional_and_omitted() {
    let absent = event();
    assert_eq!(absent.sent_to_last_content_ms, None);
    assert_eq!(absent.first_to_last_content_ms, None);
    assert_eq!(absent.route_decision_id, None);
    let base = serde_json::to_value(&absent).unwrap();
    for (field, value) in [
        ("sent_to_last_content_ms", json!(0)),
        ("first_to_last_content_ms", json!(0)),
        ("route_decision_id", json!("decision")),
    ] {
        assert!(base.get(field).is_none());
        let mut body = base.clone();
        body[field] = value;
        let decoded: StepFinished = serde_json::from_value(body.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), body);
    }
    let reported = absent
        .with_sent_to_last_content_ms(8)
        .with_first_to_last_content_ms(5)
        .with_route_decision_id("decision");
    assert_eq!(reported.sent_to_last_content_ms, Some(8));
    assert_eq!(reported.first_to_last_content_ms, Some(5));
    assert_eq!(reported.route_decision_id.as_deref(), Some("decision"));
    let bytes = serde_json::to_vec(&reported).unwrap();
    assert_eq!(decode_step_finished(SUBJECT, &bytes).unwrap(), reported);
}

#[test]
fn timings_require_unsigned_integer_milliseconds_and_string_decision() {
    let base = serde_json::to_value(event()).unwrap();
    for field in ["sent_to_last_content_ms", "first_to_last_content_ms"] {
        for value in [json!(-1), json!(0.5), json!("1"), json!(false)] {
            let mut body = base.clone();
            body[field] = value;
            assert!(serde_json::from_value::<StepFinished>(body).is_err());
        }
    }
    let mut body = base;
    body["route_decision_id"] = json!(42);
    assert!(serde_json::from_value::<StepFinished>(body).is_err());
}

#[test]
fn timings_086_bodies_round_trip_byte_for_byte() {
    // These compact bodies pin the pre-timing field order and omission rules,
    // independently of the current serializer's output.
    let bodies = [
        r#"{"runner":"runner","session_ref":"ref","log_incarnation":"log","seq":1,"run_id":"run","step_id":"step","attempt":1,"terminal_status":"completed","attempt_id":"attempt","harness":"host","session":"session","provider":"provider","model":"model","started_at_ms":10,"finished_at_ms":20}"#,
        r#"{"runner":"runner","session_ref":"ref","log_incarnation":"log","seq":1,"run_id":"run","step_id":"step","attempt":2,"terminal_status":"future_status","harness_step_ref":"record","provider_request_id":"request","harness":"host","session":"session","work_class":"interactive","provider":"provider","model":"model","input_tokens":1,"cached_input_tokens":2,"cache_write_tokens":3,"output_tokens":4,"reasoning_tokens":0,"cache_write_5m_tokens":1,"cache_write_1h_tokens":2,"reasoning_included_in_output":false,"request_body_bytes":5,"context_limit":6,"finish_reason":"stop","started_at_ms":10,"finished_at_ms":20}"#,
    ];
    for body in bodies {
        let decoded = decode_step_finished(SUBJECT, body.as_bytes()).unwrap();
        assert_eq!(decoded.sent_to_last_content_ms, None);
        assert_eq!(decoded.first_to_last_content_ms, None);
        assert_eq!(decoded.route_decision_id, None);
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), body.as_bytes());
    }
}
