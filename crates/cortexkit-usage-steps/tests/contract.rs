//! Behaviour the contract pins beyond the vector bytes: last-row-wins,
//! open value sets, lenient readers with a strict provider-side check, strict
//! requests, required row members and the builders.

use std::path::Path;

use cortexkit_usage_steps::{
    codes, Applied, BadCounts, Cursor, LatestRows, NotClosed, Notice, ReadReply, ReadRequest,
    Refusal, RequestSize, StepRow, StepSource, SubscribeRequest, Verdict, DEFAULT_READ_LIMIT,
    MAX_READ_LIMIT, PROMPT_DROP_TOKENS, STABLE_RETENTION_AT_LEAST, WARNING_RETENTION_AT_LEAST,
};

fn read(rel: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors/usage-steps-v1")
        .join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("{rel}: {err}"))
}

fn value(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap()
}

#[test]
fn latest_rows_keeps_the_last_row_per_identity() {
    let reply: ReadReply = serde_json::from_str(&read("read/reply-page.json")).unwrap();
    assert_eq!(reply.rows.len(), 8);
    let mut latest = LatestRows::new();
    latest.apply_reply(reply);
    assert_eq!(latest.len(), 7, "a repeated identity was appended");
    let kept = latest.get("opencode", "msg_ex_0001").unwrap();
    assert_eq!(kept.cursor, Cursor::new("c0000104"));
    assert_eq!(kept.finish.as_deref(), Some("tool-calls"));
    // The replaced row keeps its place: identities stay in first-seen order.
    let order: Vec<&str> = latest.iter().map(|row| row.step_ref.as_str()).collect();
    assert_eq!(order[0], "msg_ex_0001");
    assert_eq!(latest.resume_after(), Some(&Cursor::new("c0000108")));

    let last: ReadReply = serde_json::from_str(&read("read/reply-last-page.json")).unwrap();
    latest.apply_reply(last);
    assert_eq!(latest.len(), 9);
    assert_eq!(latest.resume_after(), Some(&Cursor::new("c0000110")));

    latest.clear();
    assert!(latest.is_empty());
    assert_eq!(latest.resume_after(), None);
}

#[test]
fn latest_rows_reports_added_then_replaced_and_a_repeat_is_harmless() {
    let row = StepRow::new(
        "c1",
        "codex",
        StepSource::File,
        "turn_1",
        "ses_1",
        1,
        10,
        Verdict::Unknown,
        false,
    );
    let changed = StepRow::new(
        "c2",
        "codex",
        StepSource::File,
        "turn_1",
        "ses_1",
        1,
        10,
        Verdict::ColdStart,
        false,
    );
    let other_harness = StepRow::new(
        "c3",
        "gemini",
        StepSource::File,
        "turn_1",
        "ses_1",
        1,
        10,
        Verdict::Unknown,
        false,
    );
    let mut latest = LatestRows::new();
    assert_eq!(latest.apply(row), Applied::Added);
    assert_eq!(latest.apply(changed.clone()), Applied::Replaced);
    assert_eq!(latest.apply(changed.clone()), Applied::Replaced);
    // Same step_ref under another harness is another identity.
    assert_eq!(latest.apply(other_harness), Applied::Added);
    assert_eq!(latest.len(), 2);
    assert_eq!(latest.get("codex", "turn_1"), Some(&changed));
}

#[test]
fn unknown_verdict_decodes_as_other_and_round_trips() {
    let verdict: Verdict = serde_json::from_str("\"cache_drift\"").unwrap();
    assert_eq!(verdict, Verdict::Other("cache_drift".to_owned()));
    assert!(!verdict.is_known());
    assert_eq!(serde_json::to_string(&verdict).unwrap(), "\"cache_drift\"");

    let mut row = value("read/row-every-member.json");
    row["verdict"] = "cache_drift".into();
    let row: StepRow = serde_json::from_value(row).unwrap();
    assert_eq!(row.verdict, Verdict::Other("cache_drift".to_owned()));
}

#[test]
fn wire_unknown_is_a_real_verdict_and_every_known_value_decodes_to_its_variant() {
    let expected = [
        ("cold_start", Verdict::ColdStart),
        ("stable", Verdict::Stable),
        ("warning", Verdict::Warning),
        ("bust", Verdict::Bust),
        ("full_bust", Verdict::FullBust),
        ("unknown", Verdict::Unknown),
    ];
    let names: Vec<&str> = expected.iter().map(|(wire, _)| *wire).collect();
    assert_eq!(Verdict::KNOWN, names.as_slice());
    for (wire, verdict) in expected {
        let decoded: Verdict = serde_json::from_str(&format!("\"{wire}\"")).unwrap();
        assert_eq!(decoded, verdict);
        assert!(decoded.is_known());
    }
    for (wire, size) in [
        ("smaller", RequestSize::Smaller),
        ("not_smaller", RequestSize::NotSmaller),
    ] {
        assert_eq!(RequestSize::from_wire(wire), size);
    }
    for (wire, source) in [("file", StepSource::File), ("runner", StepSource::Runner)] {
        assert_eq!(StepSource::from_wire(wire), source);
    }
}

#[test]
fn unknown_row_member_is_ignored_on_decode_and_named_by_check_closed() {
    let with_extra = value("tolerant/row-unknown-member.json");
    let row: StepRow = serde_json::from_value(with_extra.clone()).unwrap();
    assert_eq!(
        serde_json::to_string(&row).unwrap(),
        read("read/row-every-member.json")
    );
    assert_eq!(
        StepRow::check_closed(&with_extra),
        Err(NotClosed::UnknownMember {
            object: "row",
            member: "cache_hint_ms".to_owned(),
        })
    );

    // The same row inside a reply is found by the reply check too.
    let reply = serde_json::json!({"rows": [with_extra], "next": null, "through": "c9"});
    let decoded: ReadReply = serde_json::from_value(reply.clone()).unwrap();
    assert_eq!(decoded.rows.len(), 1);
    assert!(matches!(
        ReadReply::check_closed(&reply),
        Err(NotClosed::UnknownMember { object: "row", .. })
    ));

    let reply_extra = value("tolerant/reply-unknown-member.json");
    assert_eq!(
        ReadReply::check_closed(&reply_extra),
        Err(NotClosed::UnknownMember {
            object: "reply",
            member: "held_rows".to_owned(),
        })
    );
    let notice_extra = value("tolerant/notice-unknown-member.json");
    assert!(serde_json::from_value::<Notice>(notice_extra.clone()).is_ok());
    assert_eq!(
        Notice::check_closed(&notice_extra),
        Err(NotClosed::UnknownMember {
            object: "notice",
            member: "subscribers".to_owned(),
        })
    );
    assert_eq!(
        StepRow::check_closed(&serde_json::json!([])),
        Err(NotClosed::NotAnObject { object: "row" })
    );
}

#[test]
fn check_closed_accepts_every_published_shape() {
    for file in [
        "read/reply-page.json",
        "read/reply-last-page.json",
        "read/reply-empty.json",
    ] {
        assert_eq!(ReadReply::check_closed(&value(file)), Ok(()), "{file}");
    }
    for file in ["read/row-every-member.json", "read/row-required-only.json"] {
        assert_eq!(StepRow::check_closed(&value(file)), Ok(()), "{file}");
    }
    assert_eq!(
        Notice::check_closed(&value("subscribe/notice.json")),
        Ok(())
    );
}

fn keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

#[test]
fn published_member_lists_match_what_the_row_encodes() {
    let mut every: Vec<String> = StepRow::REQUIRED_MEMBERS
        .iter()
        .chain(StepRow::OPTIONAL_MEMBERS)
        .map(|m| m.to_string())
        .collect();
    every.sort();
    let full: StepRow = serde_json::from_str(&read("read/row-every-member.json")).unwrap();
    // Keep the published legacy-byte fixture unchanged while exercising every
    // current optional member in the schema completeness check.
    let full = full
        .with_service_tier_applied(
            cortexkit_role_llm_runner::service_tier::ServiceTier::Fast.as_str(),
        )
        .with_service_tier_served("fast");
    assert_eq!(keys(&serde_json::to_value(&full).unwrap()), every);

    let mut required: Vec<String> = StepRow::REQUIRED_MEMBERS
        .iter()
        .map(|m| m.to_string())
        .collect();
    required.sort();
    let minimal: StepRow = serde_json::from_str(&read("read/row-required-only.json")).unwrap();
    assert_eq!(keys(&serde_json::to_value(&minimal).unwrap()), required);
}

#[test]
fn a_row_missing_any_required_member_is_refused() {
    let full = value("read/row-every-member.json");
    for member in StepRow::REQUIRED_MEMBERS {
        let mut row = full.clone();
        row.as_object_mut().unwrap().remove(*member);
        assert!(
            serde_json::from_value::<StepRow>(row).is_err(),
            "a row without {member} decoded"
        );
    }
    for member in StepRow::OPTIONAL_MEMBERS {
        let mut row = full.clone();
        row.as_object_mut().unwrap().remove(*member);
        assert!(
            serde_json::from_value::<StepRow>(row).is_ok(),
            "a row without optional {member} was refused"
        );
    }
}

#[test]
fn requests_refuse_an_unknown_member_and_accept_null_optionals() {
    assert!(serde_json::from_str::<SubscribeRequest>("{\"after\":\"c1\"}").is_err());
    assert!(serde_json::from_str::<SubscribeRequest>("{}").is_ok());
    assert!(serde_json::from_str::<ReadRequest>("{\"after\":\"c1\",\"unknown\":\"x\"}").is_err());
    let nulls: ReadRequest = serde_json::from_str("{\"after\":null,\"limit\":null}").unwrap();
    assert_eq!(nulls, ReadRequest::new());
}

#[test]
fn session_members_round_trip_and_absent_ones_stay_out_of_the_bytes() {
    let filtered: ReadRequest = serde_json::from_str(&read("read/request-session.json")).unwrap();
    assert_eq!(filtered.harness.as_deref(), Some("opencode"));
    assert_eq!(filtered.session.as_deref(), Some("ses_demo01"));
    assert_eq!(
        serde_json::to_string(&filtered).unwrap(),
        read("read/request-session.json")
    );

    let filtered: SubscribeRequest =
        serde_json::from_str(&read("subscribe/request-session.json")).unwrap();
    assert_eq!(filtered.harness.as_deref(), Some("opencode"));
    assert_eq!(filtered.session.as_deref(), Some("ses_demo01"));
    assert_eq!(
        serde_json::to_string(&filtered).unwrap(),
        read("subscribe/request-session.json")
    );

    // Null is accepted for both members, as for `after` and `limit`.
    let nulls: ReadRequest = serde_json::from_str("{\"harness\":null,\"session\":null}").unwrap();
    assert_eq!(nulls, ReadRequest::new());
    let nulls: SubscribeRequest =
        serde_json::from_str("{\"harness\":null,\"session\":null}").unwrap();
    assert_eq!(nulls, SubscribeRequest::new());

    // Absent, never null, in what this crate encodes.
    assert_eq!(
        serde_json::to_string(&ReadRequest::new().after("c0000100").with_limit(200)).unwrap(),
        "{\"after\":\"c0000100\",\"limit\":200}"
    );
    assert_eq!(
        serde_json::to_string(&ReadRequest::new().for_session("opencode", "ses_demo01")).unwrap(),
        "{\"harness\":\"opencode\",\"session\":\"ses_demo01\"}"
    );
    assert_eq!(
        serde_json::to_string(&SubscribeRequest::new().for_session("opencode", "ses_demo01"))
            .unwrap(),
        read("subscribe/request-session.json")
    );
}

#[test]
fn session_pairing_accepts_both_or_neither_and_refuses_either_alone() {
    let harness_only: ReadRequest = serde_json::from_str("{\"harness\":\"opencode\"}").unwrap();
    let session_only: ReadRequest = serde_json::from_str("{\"session\":\"ses_demo01\"}").unwrap();
    assert!(harness_only.check_pairing().is_err());
    assert!(session_only.check_pairing().is_err());
    assert_eq!(ReadRequest::new().check_pairing(), Ok(()));
    assert_eq!(
        ReadRequest::new()
            .for_session("opencode", "ses_demo01")
            .check_pairing(),
        Ok(())
    );
    let nulls: ReadRequest = serde_json::from_str("{\"harness\":null,\"session\":null}").unwrap();
    assert_eq!(nulls.check_pairing(), Ok(()));

    let harness_only: SubscribeRequest =
        serde_json::from_str("{\"harness\":\"opencode\"}").unwrap();
    let session_only: SubscribeRequest =
        serde_json::from_str("{\"session\":\"ses_demo01\"}").unwrap();
    assert!(harness_only.check_pairing().is_err());
    assert!(session_only.check_pairing().is_err());
    assert_eq!(SubscribeRequest::new().check_pairing(), Ok(()));
    assert_eq!(
        SubscribeRequest::new()
            .for_session("opencode", "ses_demo01")
            .check_pairing(),
        Ok(())
    );
}

/// The vectors a provider that predates the session filter pins. Adding the
/// filter must not change one byte of what an unfiltered request encodes to.
#[test]
fn unfiltered_requests_encode_to_the_bytes_they_always_did() {
    for (request, pinned) in [
        (serde_json::to_string(&ReadRequest::new()).unwrap(), "{}"),
        (
            serde_json::to_string(&ReadRequest::new().after("c0000100").with_limit(200)).unwrap(),
            "{\"after\":\"c0000100\",\"limit\":200}",
        ),
        (
            serde_json::to_string(&ReadRequest::new().with_limit(1_000)).unwrap(),
            "{\"limit\":1000}",
        ),
        (
            serde_json::to_string(&SubscribeRequest::new()).unwrap(),
            "{}",
        ),
        (
            serde_json::to_string(&serde_json::from_str::<ReadRequest>("{}").unwrap()).unwrap(),
            "{}",
        ),
    ] {
        assert_eq!(request, pinned);
    }
    assert_eq!(read("read/request-from-oldest.json"), "{}");
    assert_eq!(read("subscribe/request.json"), "{}");
    assert_eq!(
        read("read/request-after.json"),
        "{\"after\":\"c0000100\",\"limit\":200}"
    );
    assert_eq!(read("read/request-limit-only.json"), "{\"limit\":1000}");
    assert_eq!(
        read("tolerant/read-request-null-members.json"),
        "{\"after\":null,\"limit\":null}"
    );
}

#[test]
fn builders_produce_the_vector_bytes() {
    let request = ReadRequest::new().after("c0000100").with_limit(200);
    assert_eq!(
        serde_json::to_string(&request).unwrap(),
        read("read/request-after.json")
    );
    assert_eq!(
        serde_json::to_string(&SubscribeRequest::new()).unwrap(),
        read("subscribe/request.json")
    );
    assert_eq!(
        serde_json::to_string(&Notice::new("c0000110")).unwrap(),
        read("subscribe/notice.json")
    );
    let expired = Refusal::cursor_expired("c0000110", "cursor is older than the feed holds");
    assert_eq!(
        serde_json::to_string(&expired).unwrap(),
        read("refusals/cursor_expired.json")
    );
    assert_eq!(expired.through(), Some(Cursor::new("c0000110")));
    let lagged = Refusal::lagged("c0000104", "subscriber fell behind");
    assert_eq!(
        serde_json::to_string(&lagged).unwrap(),
        read("endings/lagged.json")
    );
    let cancelled = Refusal::cancelled("subscription cancelled by the caller");
    assert_eq!(
        serde_json::to_string(&cancelled).unwrap(),
        read("endings/cancelled.json")
    );
    assert_eq!(cancelled.through(), None);
    let denied = Refusal::new(
        codes::NOT_PERMITTED,
        "this caller may not read the usage feed",
    );
    assert_eq!(
        serde_json::to_string(&denied).unwrap(),
        read("refusals/not_permitted.json")
    );
    assert_eq!(denied.through(), None);

    let row = StepRow::new(
        "c0000109",
        "example-runner",
        StepSource::File,
        "run_ex_8#step_ex_3#1",
        "ses_ex_f",
        1_792_880_031_000,
        64_300,
        Verdict::Warning,
        true,
    )
    .with_run_id("run_ex_8")
    .with_provider("example-provider")
    .with_model("example-model-large")
    .with_input(2_500)
    .with_cache_read(60_000)
    .with_cache_write(1_800)
    .with_cache_write_5m(1_800)
    .with_cache_write_1h(0)
    .with_output(420)
    .with_reasoning(60)
    .with_finish("length")
    .with_context_limit(200_000)
    .with_request_body_len(250_000)
    .with_retention(60_000.0 / 70_000.0)
    .with_request_size(RequestSize::NotSmaller);
    assert_eq!(
        serde_json::to_string(&row).unwrap(),
        read("read/row-every-member.json")
    );
    assert_eq!(row.identity(), ("example-runner", "run_ex_8#step_ex_3#1"));

    let reply = ReadReply::new(vec![], None, "c0000110");
    assert_eq!(
        serde_json::to_string(&reply).unwrap(),
        read("read/reply-empty.json")
    );
    assert_eq!(reply.resume_after(), &Cursor::new("c0000110"));
}

#[test]
fn check_counts_names_a_row_whose_reasoning_exceeds_its_output() {
    let row = |reasoning: Option<u64>, output: Option<u64>| {
        let mut row = StepRow::new(
            "c1",
            "codex",
            StepSource::File,
            "turn_1",
            "ses_1",
            1,
            10,
            Verdict::Unknown,
            false,
        );
        row.reasoning = reasoning;
        row.output = output;
        row
    };
    assert_eq!(
        row(Some(801), Some(800)).check_counts(),
        Err(BadCounts::ReasoningExceedsOutput {
            harness: "codex".to_owned(),
            step_ref: "turn_1".to_owned(),
            reasoning: 801,
            output: 800,
        })
    );
    // Reasoning is inside output, so equal counts are fine.
    assert_eq!(row(Some(800), Some(800)).check_counts(), Ok(()));
    assert_eq!(row(Some(800), None).check_counts(), Ok(()));
    assert_eq!(row(None, Some(800)).check_counts(), Ok(()));

    // Readers still decode a row that breaks the rule.
    let mut bad = value("read/row-every-member.json");
    bad["reasoning"] = 5_000.into();
    let decoded: StepRow = serde_json::from_value(bad).unwrap();
    assert!(decoded.check_counts().is_err());
}

#[test]
fn retention_above_one_is_kept_exactly() {
    let reply: ReadReply = serde_json::from_str(&read("read/reply-page.json")).unwrap();
    let row = reply
        .rows
        .iter()
        .find(|row| row.cursor == Cursor::new("c0000102"))
        .unwrap();
    assert_eq!(row.retention, Some(50_000.0 / 48_500.0));
    assert!(row.retention.unwrap() > 1.0);
    assert_eq!(row.verdict, Verdict::Stable);
}

#[test]
fn thresholds_and_limits_are_the_contract_values() {
    assert_eq!(STABLE_RETENTION_AT_LEAST, 0.95);
    assert_eq!(WARNING_RETENTION_AT_LEAST, 0.80);
    assert_eq!(PROMPT_DROP_TOKENS, 15_000);
    assert_eq!(DEFAULT_READ_LIMIT, 200);
    assert_eq!(MAX_READ_LIMIT, 1_000);
}
