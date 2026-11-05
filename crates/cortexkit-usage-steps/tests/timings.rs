use cortexkit_usage_steps::{BadCounts, StepRow, StepSource, Verdict};
use serde_json::{json, Value};

const TIMING_MEMBERS: &[&str] = &["sent_to_last_content_ms", "first_to_last_content_ms"];

fn row() -> StepRow {
    StepRow::new(
        "c1",
        "example-runner",
        StepSource::Runner,
        "attempt_ex",
        "session_ex",
        1,
        10,
        Verdict::Unknown,
        false,
    )
}

#[test]
fn timings_preserve_0_2_3_row_bytes_and_omit_unknown_members() {
    for legacy in [
        include_str!("../test-vectors/usage-steps-v1/read/row-every-member.json"),
        include_str!("../test-vectors/usage-steps-v1/read/row-required-only.json"),
        // Service tiers were already supported in 0.2.3, including unfamiliar
        // provider-served spellings that readers must preserve.
        r#"{"cursor":"c1","harness":"example-runner","source":"runner","step_ref":"attempt_ex","session":"session_ex","occurred_at_ms":1,"service_tier_applied":"fast","service_tier_served":"future-tier","prompt":10,"verdict":"unknown","prompt_dropped":false}"#,
    ] {
        let decoded: StepRow = serde_json::from_str(legacy).unwrap();
        assert_eq!(decoded.sent_to_last_content_ms, None);
        assert_eq!(decoded.first_to_last_content_ms, None);
        assert_eq!(decoded.route_decision_id, None);
        assert_eq!(serde_json::to_string(&decoded).unwrap(), legacy);
    }

    let constructed = row();
    assert_eq!(constructed.sent_to_last_content_ms, None);
    assert_eq!(constructed.first_to_last_content_ms, None);
    assert_eq!(constructed.route_decision_id, None);
    let encoded = serde_json::to_value(constructed).unwrap();
    for member in TIMING_MEMBERS.iter().copied().chain(["route_decision_id"]) {
        assert!(
            encoded.get(member).is_none(),
            "unknown {member} was emitted"
        );
    }
}

#[test]
fn timings_optional_members_round_trip_independently_including_measured_zero() {
    let base = serde_json::to_value(row()).unwrap();
    for mask in 0..8 {
        for duration in [0, 500, u64::MAX] {
            let mut raw = base.clone();
            if mask & 1 != 0 {
                raw["sent_to_last_content_ms"] = duration.into();
            }
            if mask & 2 != 0 {
                raw["first_to_last_content_ms"] = duration.into();
            }
            if mask & 4 != 0 {
                raw["route_decision_id"] = "decision_ex".into();
            }
            assert_eq!(StepRow::check_closed(&raw), Ok(()));
            let decoded: StepRow = serde_json::from_value(raw.clone()).unwrap();
            assert_eq!(decoded.check_counts(), Ok(()));
            assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
        }
    }

    let mut nulls = base.clone();
    for member in TIMING_MEMBERS.iter().copied().chain(["route_decision_id"]) {
        nulls[member] = Value::Null;
    }
    let decoded: StepRow = serde_json::from_value(nulls).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), base);
}

#[test]
fn timings_decode_only_unsigned_integer_milliseconds_and_a_string_decision() {
    for member in TIMING_MEMBERS {
        for invalid in [json!(-1), json!(0.5), json!("1"), json!(true)] {
            let mut raw = serde_json::to_value(row()).unwrap();
            raw[*member] = invalid;
            assert!(
                serde_json::from_value::<StepRow>(raw).is_err(),
                "invalid {member} decoded"
            );
        }
    }
    let mut raw = serde_json::to_value(row()).unwrap();
    raw["route_decision_id"] = json!(123);
    assert!(serde_json::from_value::<StepRow>(raw).is_err());
}

#[test]
fn timings_check_counts_names_a_content_duration_exceeding_the_send_duration() {
    for (first, sent) in [(1, 0), (501, 500), (u64::MAX, u64::MAX - 1)] {
        let mut raw = serde_json::to_value(row()).unwrap();
        raw["first_to_last_content_ms"] = first.into();
        raw["sent_to_last_content_ms"] = sent.into();
        // Readers remain tolerant of inconsistent measurements; providers use
        // check_counts to catch a duration that cannot fit within the call.
        let decoded: StepRow = serde_json::from_value(raw).unwrap();
        let expected = BadCounts::FirstToLastContentExceedsSentToLastContent {
            harness: "example-runner".to_owned(),
            step_ref: "attempt_ex".to_owned(),
            first_to_last_content_ms: first,
            sent_to_last_content_ms: sent,
        };
        assert_eq!(decoded.check_counts(), Err(expected.clone()));
        assert_eq!(
            expected.to_string(),
            format!(
                "row example-runner attempt_ex: first_to_last_content_ms {first} exceeds sent_to_last_content_ms {sent}"
            )
        );
    }
}

#[test]
fn timings_check_counts_accepts_equal_shorter_and_unknown_durations() {
    for (first, sent) in [
        (Some(0), Some(0)),
        (Some(0), Some(1)),
        (Some(400), Some(500)),
        (Some(u64::MAX), Some(u64::MAX)),
        (Some(u64::MAX), None),
        (None, Some(0)),
        (None, None),
    ] {
        let mut measured = row();
        measured.first_to_last_content_ms = first;
        measured.sent_to_last_content_ms = sent;
        assert_eq!(measured.check_counts(), Ok(()), "{first:?}, {sent:?}");
    }
}
