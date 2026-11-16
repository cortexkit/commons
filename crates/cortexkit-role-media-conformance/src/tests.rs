mod fake;
mod stream_realtime;

use crate::{
    checks::{run_case, Outcome, CASES},
    fixtures::{sha256, AudioFixture},
    peers::{RecordingRoutingPeer, RecordingVendor},
    rule_map::{validate_rule_map, RULE_MAP},
    subject::*,
};
use async_trait::async_trait;
use fake::Fake;
use serde_json::{json, Value};
const CONTRACT: &str = include_str!("../../cortexkit-role-media/CONTRACT.md");

#[tokio::test]
async fn media_kit_full_featured_case_count_unchanged() {
    let mut passed = 0;
    for &case in CASES {
        assert_eq!(
            run_case(&Fake::new(None, true, true), case).await,
            Outcome::Passed,
            "{case}"
        );
        passed += 1;
    }
    assert_eq!(passed, 90, "full-featured case count from 0.2.9");
}

#[tokio::test]
async fn media_kit_local_no_usage_feed_does_not_read_usage() {
    let subject = Fake::new(None, true, true)
        .with_local_provider(false)
        .without_usage_feed();
    assert_eq!(
        run_case(&subject, "submit_idempotent_on_client_job_id").await,
        Outcome::Passed
    );
    assert_eq!(subject.usage_hook_counts(), (0, 0));
}

#[tokio::test]
async fn media_kit_fixed_catalog_non_stream_cases() {
    let mut passed = 0;
    let mut not_applicable = 0;
    for &case in CASES
        .iter()
        .filter(|case| !crate::checks::STREAM_REALTIME_CASES.contains(case))
    {
        let subject =
            Fake::new(None, true, true).with_fixed_catalog(Some(32 * 1024 * 1024), Some(3_600_000));
        match run_case(&subject, case).await {
            Outcome::Passed => passed += 1,
            Outcome::NotApplicable(reason) => {
                assert!(!reason.is_empty(), "{case}");
                not_applicable += 1;
            }
            outcome => panic!("fixed-catalog {case}: {outcome:?}"),
        }
        assert_eq!(subject.usage_hook_counts(), (0, 0), "{case}");
        let engines = subject.engine_declaration().await.unwrap();
        assert_eq!(engines.len(), 1, "{case}");
        assert_eq!(engines[0].kind.as_str(), "transcribe", "{case}");
        assert!(engines[0].local, "{case}");
    }
    assert_eq!((passed, not_applicable), (21, 25));
    assert_eq!(passed + not_applicable, 46);
}

#[tokio::test]
async fn media_kit_fixed_catalog_inapplicable_reasons() {
    for (case, reason) in [
        ("wire_extension", "configure_engines"),
        ("describe_matches_engine_declaration", "configure_engines"),
        ("describe_checker_codes", "configure_engines"),
        ("input_too_large", "16 MiB"),
        ("empty_output_is_a_failure", "synthesize, image, video"),
        ("chunked_result_checksum", "synthesize"),
        ("usage_feed_oversize_entry_acks", "usage_feed"),
    ] {
        let subject =
            Fake::new(None, true, true).with_fixed_catalog(Some(32 * 1024 * 1024), Some(3_600_000));
        let outcome = run_case(&subject, case).await;
        assert!(
            matches!(outcome, Outcome::NotApplicable(ref message) if message.contains(reason)),
            "{case}: {outcome:?}"
        );
        assert_eq!(subject.usage_hook_counts(), (0, 0), "{case}");
    }
}

#[tokio::test]
async fn media_kit_fixed_catalog_input_limits_follow_describe() {
    for (bytes, duration) in [
        (Some(131_072), Some(5_000)),
        (Some(3 * 1024 * 1024), None),
        (None, Some(5_000)),
    ] {
        let subject = Fake::new(None, true, true).with_fixed_catalog(bytes, duration);
        assert_eq!(
            run_case(&subject, "input_too_large").await,
            Outcome::Passed,
            "bytes={bytes:?}, duration={duration:?}"
        );
        if duration.is_some() {
            let inputs = subject.engine_inputs("transcribe").await.unwrap();
            assert_eq!(inputs.len(), 1);
            assert_eq!(inputs[0]["language"], "ja");
        }
        assert_eq!(subject.usage_hook_counts(), (0, 0));
    }
    for (bytes, duration, reason) in [
        (None, None, "no input limits"),
        (None, Some(u64::MAX), "cannot be exceeded"),
    ] {
        let subject = Fake::new(None, true, true).with_fixed_catalog(bytes, duration);
        let outcome = run_case(&subject, "input_too_large").await;
        assert!(
            matches!(outcome, Outcome::NotApplicable(ref message) if message.contains(reason)),
            "{outcome:?}"
        );
    }
}

#[tokio::test]
async fn media_kit_fixed_catalog_undeclared_kind_field_is_checked() {
    let subject = Fake::new(Some("undeclared_kind_field"), true, true)
        .with_fixed_catalog(Some(32 * 1024 * 1024), Some(3_600_000));
    let outcome = run_case(&subject, "submit_refuses_outside_declared_facts").await;
    assert!(
        matches!(outcome, Outcome::Failed(ref message) if message.starts_with("submit_refuses_outside_declared_facts:") && message.contains("expected field kind")),
        "{outcome:?}"
    );
}

#[tokio::test]
async fn media_kit_synthetic_setup_errors_are_failures() {
    for case in [
        "wire_extension",
        "describe_matches_engine_declaration",
        "describe_checker_codes",
    ] {
        let subject = Fake::new(Some("configure_engines_failure"), true, true);
        assert_eq!(
            run_case(&subject, case).await,
            Outcome::Failed(format!("{case}: engine configuration failed"))
        );
    }
}

#[tokio::test]
async fn media_kit_wrapped_reply_envelope_passes() {
    for case in ["reply_envelope", "stream_reply_envelope"] {
        assert_eq!(
            run_case(&Fake::new(None, true, true), case).await,
            Outcome::Passed,
            "{case}"
        );
    }
}

#[tokio::test]
async fn media_kit_bare_reply_envelope_is_rejected() {
    for case in ["reply_envelope", "stream_reply_envelope"] {
        let bad = run_case(&Fake::new(Some(case), true, true), case).await;
        assert!(
            matches!(bad, Outcome::Failed(ref reason) if reason.starts_with(&format!("{case}:")) && reason.contains("bare_answer")),
            "{case}: {bad:?}"
        );
    }
}

#[tokio::test]
async fn media_kit_reply_envelope_extra_keys_are_rejected() {
    for (case, fault) in [
        ("reply_envelope", "reply_envelope_extra_keys"),
        ("stream_reply_envelope", "stream_reply_envelope_extra_keys"),
    ] {
        let bad = run_case(&Fake::new(Some(fault), true, true), case).await;
        assert!(
            matches!(bad, Outcome::Failed(ref reason) if reason.starts_with(&format!("{case}:")) && reason.contains("extra_keys")),
            "{case}: {bad:?}"
        );
    }
}

#[tokio::test]
async fn media_kit_describe_declaration_distinguishes_providers() {
    let case = "describe_matches_engine_declaration";
    assert_eq!(
        run_case(&Fake::new(None, true, true), case).await,
        Outcome::Passed
    );
    for fault in [
        "describe_declaration_omits_provider",
        "describe_declaration_duplicates_provider",
    ] {
        let bad = run_case(&Fake::new(Some(fault), true, true), case).await;
        assert!(
            matches!(bad, Outcome::Failed(ref reason) if reason.starts_with(&format!("{case}:")) && reason.contains("describe/declaration facts differ")),
            "{fault}: {bad:?}"
        );
    }
}

macro_rules! case_tests {
    ($($name:ident),* $(,)?) => {$(
        #[tokio::test]
        async fn $name() {
            let case=stringify!($name).strip_prefix("media_kit_core_").unwrap();
            assert_eq!(run_case(&Fake::new(None,true,true),case).await,Outcome::Passed,"conforming {case}");
            let bad=run_case(&Fake::new(Some(case),true,true),case).await;
            assert!(matches!(bad,Outcome::Failed(ref reason) if reason.starts_with(&format!("{case}:"))),"faulty {case}: {bad:?}");
        }
    )*};
}
case_tests! {
    media_kit_core_wire_extension,
    media_kit_core_realtime_watch,
    media_kit_core_submit_idempotent_on_client_job_id,
    media_kit_core_submit_refuses_client_job_id_reuse,
    media_kit_core_refused_submit_not_remembered,
    media_kit_core_submit_refuses_outside_declared_facts,
    media_kit_core_input_too_large,
    media_kit_core_remote_submit_requires_decision,
    media_kit_core_client_job_id_bounds,
    media_kit_core_describe_matches_engine_declaration,
    media_kit_core_describe_checker_codes,
    media_kit_core_deadline_counts_queue_time,
    media_kit_core_refusal_class_decides_retry,
    media_kit_core_empty_transcript_is_a_failure,
    media_kit_core_empty_output_is_a_failure,
    media_kit_core_usage_absent_is_not_zero,
    media_kit_core_usage_no_rounding,
    media_kit_core_usage_input_image,
    media_kit_core_usage_output_image,
    media_kit_core_usage_output_video,
    media_kit_core_usage_cache_read,
    media_kit_core_usage_request_derived,
    media_kit_core_usage_record_bounds,
    media_kit_core_usage_feed_redelivers_until_ack,
    media_kit_core_usage_feed_admits_only_reader,
    media_kit_core_usage_ack_beyond_written_refused,
    media_kit_core_usage_ack_answer,
    media_kit_core_usage_feed_oversize_entry_acks,
    media_kit_core_failure_identical_across_surfaces,
    media_kit_core_usage_record_per_attempt,
    media_kit_core_restart_keeps_terminal_jobs,
    media_kit_core_cancel_then_restart_records_cancelled,
    media_kit_core_running_restart_records_failure,
    media_kit_core_chunked_result_checksum,
    media_kit_core_upload_resumes_at_received,
    media_kit_core_upload_gone_after_restart,
    media_kit_core_committed_upload_is_object,
    media_kit_core_oversize_output_is_object,
    media_kit_core_objects_retained_for_one_hour,
    media_kit_core_cancel_after_done_reports_done,
    media_kit_core_cancel_never_changes_terminal_status,
    media_kit_core_foreign_job_is_unknown,
    media_kit_core_routing_admission_per_attempt,
    media_kit_core_renewal_lapse_ends_work,
}

const LOCAL_JOB_CASES: &[&str] = &[
    "submit_idempotent_on_client_job_id",
    "submit_refuses_client_job_id_reuse",
    "submit_refuses_outside_declared_facts",
    "input_too_large",
    "client_job_id_bounds",
    "describe_matches_engine_declaration",
    "describe_checker_codes",
    "deadline_counts_queue_time",
    "refusal_class_decides_retry",
    "empty_transcript_is_a_failure",
    "empty_output_is_a_failure",
    "usage_feed_oversize_entry_acks",
    "failure_identical_across_surfaces",
    "usage_record_per_attempt",
    "restart_keeps_terminal_jobs",
    "cancel_then_restart_records_cancelled",
    "running_restart_records_failure",
    "chunked_result_checksum",
    "upload_resumes_at_received",
    "upload_gone_after_restart",
    "committed_upload_is_object",
    "oversize_output_is_object",
    "objects_retained_for_one_hour",
    "cancel_after_done_reports_done",
    "cancel_never_changes_terminal_status",
    "foreign_job_is_unknown",
];

#[tokio::test]
async fn media_kit_local_job_cases_pass_with_optional_records() {
    assert!(!Capabilities::new().local_provider);
    for records in [false, true] {
        for capabilities in [false, true] {
            for &case in LOCAL_JOB_CASES {
                if case == "running_restart_records_failure" && !capabilities {
                    continue;
                }
                let subject =
                    Fake::new(None, capabilities, capabilities).with_local_provider(records);
                assert_eq!(
                    run_case(&subject, case).await,
                    Outcome::Passed,
                    "local {case}, records={records}, retry/restart={capabilities}"
                );
                let declared = subject.engine_declaration().await.unwrap();
                assert!(declared.iter().all(|engine| {
                    engine.kind.group() != Some(cortexkit_role_media::describe::Group::Jobs)
                        || engine.local
                }));
                if !records {
                    let feed = subject
                        .call_as("reader", Request::new("usage.read", json!({"limit":1000})))
                        .await
                        .unwrap();
                    assert_eq!(feed["result"]["entries"], json!([]), "local {case}");
                }
            }
        }
    }
}

#[tokio::test]
async fn media_kit_local_mode_preserves_stream_and_realtime_cases() {
    for case in [
        "stream_restart",
        "calls_scoped_by_principal",
        "stream_realtime_admission_lifecycle",
    ] {
        let subject = Fake::new(None, true, true).with_local_provider(true);
        assert_eq!(run_case(&subject, case).await, Outcome::Passed, "{case}");
    }
}

#[tokio::test]
async fn media_kit_local_valid_usage_record_is_checked() {
    let subject = Fake::new(None, true, true).with_local_provider(true);
    assert_eq!(
        run_case(&subject, "submit_idempotent_on_client_job_id").await,
        Outcome::Passed
    );
    let feed = subject
        .call_as("reader", Request::new("usage.read", json!({"limit":1000})))
        .await
        .unwrap();
    let entries = feed["result"]["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["record"]["charge_basis"], "local");
    assert!(entries[0]["record"].get("credential_ref").is_none());
}

#[tokio::test]
async fn media_kit_local_malformed_record_is_rejected() {
    for case in [
        "empty_transcript_is_a_failure",
        "submit_idempotent_on_client_job_id",
        "failure_identical_across_surfaces",
        "usage_record_per_attempt",
        "cancel_then_restart_records_cancelled",
        "running_restart_records_failure",
    ] {
        let subject =
            Fake::new(Some("local_malformed_usage"), true, true).with_local_provider(true);
        let bad = run_case(&subject, case).await;
        assert_eq!(
            bad,
            Outcome::Failed(format!("{case}: invalid_usage_record: charge_basis")),
            "local {case}"
        );
    }
}

#[tokio::test]
async fn media_kit_local_remote_only_cases_are_inapplicable() {
    for (cases, reason) in [
        (
            &["remote_submit_requires_decision"][..],
            "local jobs do not require a routing decision",
        ),
        (
            &["refused_submit_not_remembered"][..],
            "this case injects a remote routing admission refusal",
        ),
        (
            &["routing_admission_per_attempt"][..],
            "local jobs do not acquire or renew routing admissions",
        ),
        (
            &["renewal_lapse_ends_work"][..],
            "local jobs have no routing admission lease to lapse",
        ),
        (
            &[
                "usage_absent_is_not_zero",
                "usage_no_rounding",
                "usage_input_image",
                "usage_output_image",
                "usage_output_video",
                "usage_cache_read",
                "usage_request_derived",
                "usage_quantity_bounds",
                "usage_quantity_coefficient_bounds",
                "usage_record_bounds",
            ][..],
            "this case requires a remote job usage record and vendor usage metadata",
        ),
        (
            &[
                "usage_feed_redelivers_until_ack",
                "usage_feed_admits_only_reader",
                "usage_ack_beyond_written_refused",
                "usage_ack_answer",
            ][..],
            "this case requires a persisted job usage record; local job records are optional",
        ),
    ] {
        for case in cases {
            let subject = Fake::new(None, true, true).with_local_provider(false);
            assert_eq!(
                run_case(&subject, case).await,
                Outcome::NotApplicable(format!("local_provider is true: {reason}")),
                "{case}"
            );
        }
    }
}

#[tokio::test]
async fn media_kit_core_usage_quantity_bounds() {
    let case = "usage_quantity_bounds";
    assert_eq!(
        run_case(&Fake::new(None, true, true), case).await,
        Outcome::Passed
    );
    assert_eq!(
        run_case(&Fake::new(Some(case), true, true), case).await,
        Outcome::Failed(format!("{case}: invalid_usage_record: quantity"))
    );
}

#[tokio::test]
async fn media_kit_core_usage_quantity_coefficient_bounds() {
    let case = "usage_quantity_coefficient_bounds";
    assert_eq!(
        run_case(&Fake::new(None, true, true), case).await,
        Outcome::Passed
    );
    assert_eq!(
        run_case(&Fake::new(Some(case), true, true), case).await,
        Outcome::Failed(format!("{case}: invalid_usage_record: quantity"))
    );
}

#[test]
fn media_kit_rule_map_covers_all_pinned_ids() {
    let map = validate_rule_map(CONTRACT, RULE_MAP, CASES, |_| true).unwrap();
    let mapped: std::collections::BTreeSet<_> = map
        .rules
        .iter()
        .flat_map(|r| r.cases.iter().map(String::as_str))
        .collect();
    assert_eq!(mapped, CASES.iter().copied().collect());
    // These rules use focused incorrect behavior, not an adapter that fails setup.
    for (id, case) in [
        ("media.wire.reply", "reply_envelope"),
        ("media.wire.reply", "stream_reply_envelope"),
        ("media.objects.read", "chunked_result_checksum"),
        ("media.output.nonempty", "empty_transcript_is_a_failure"),
        ("media.usage.decimal", "usage_no_rounding"),
        ("media.usage.decimal", "usage_input_image"),
        ("media.usage.decimal", "usage_output_image"),
        ("media.usage.decimal", "usage_output_video"),
        ("media.usage.decimal", "usage_cache_read"),
        ("media.usage.decimal", "usage_request_derived"),
        ("media.usage.decimal", "usage_quantity_bounds"),
        ("media.usage.decimal", "usage_quantity_coefficient_bounds"),
    ] {
        let rule = map.rules.iter().find(|r| r.id == id).unwrap();
        assert!(rule.bug_shaped && rule.cases.iter().any(|v| v == case));
    }
}

#[tokio::test]
async fn media_kit_wire_unknown_member_is_independently_checked() {
    let bad = run_case(
        &Fake::new(Some("wire_unknown_member"), true, true),
        "wire_extension",
    )
    .await;
    assert!(
        matches!(bad, Outcome::Failed(ref reason) if reason.starts_with("wire_extension:") && reason.contains("accepted instead of invalid_params")),
        "{bad:?}"
    );
}

#[tokio::test]
async fn media_kit_watch_scope_is_independently_checked() {
    let bad = run_case(
        &Fake::new(Some("realtime_watch_scope"), true, true),
        "realtime_watch",
    )
    .await;
    assert!(
        matches!(bad, Outcome::Failed(ref reason) if reason.starts_with("realtime_watch:") && reason.contains("foreign principal or unknown call")),
        "{bad:?}"
    );
}

#[test]
fn media_kit_rule_map_rejects_each_unmapped_pinned_id() {
    let map: Value = serde_json::from_str(RULE_MAP).unwrap();
    for (index, rule) in map["rules"].as_array().unwrap().iter().enumerate() {
        let mut missing = map.clone();
        missing["rules"].as_array_mut().unwrap().remove(index);
        assert_eq!(
            validate_rule_map(CONTRACT, &missing.to_string(), CASES, |_| true).unwrap_err(),
            format!("unmapped pinned id {}", rule["id"].as_str().unwrap())
        );
    }
}

#[test]
fn media_kit_rule_map_rejects_each_mismatch() {
    let mutate = |f: fn(&mut Value)| {
        let mut v: Value = serde_json::from_str(RULE_MAP).unwrap();
        f(&mut v);
        serde_json::to_string(&v).unwrap()
    };
    let invalid = [
        mutate(|v| {
            v["rules"].as_array_mut().unwrap().remove(0);
        }),
        mutate(|v| {
            v["rules"][0]["id"] = json!("media.not_pinned");
        }),
        mutate(|v| {
            v["rules"][0]["cases"] = json!(["missing_case"]);
        }),
        mutate(|v| {
            v["rules"][0]["not_kit_checked"] = json!("not_observable");
            v["rules"][0]["cases"] = json!([]);
        }),
        mutate(|v| {
            v["rules"][0]["not_kit_checked"] = json!("unknown_reason");
        }),
        mutate(|v| {
            let r = v["rules"][0].clone();
            v["rules"].as_array_mut().unwrap().push(r);
        }),
    ];
    for bad in invalid {
        assert!(validate_rule_map(CONTRACT, &bad, CASES, |_| true).is_err());
    }
    assert!(validate_rule_map(
        &format!("{CONTRACT}\n- [pinned] a rule without an id\n"),
        RULE_MAP,
        CASES,
        |_| true
    )
    .is_err());
    let fixture="- [pinned] `media.test.rule`: test\n- [informative] `media.test.info`: not mapped\n## Rules not checked by the provider conformance kit\n- [informative] `media.kit.exemptions`: `media.test.rule` is `caller_side`.";
    let map=json!({"version":1,"rules":[{"id":"media.test.rule","not_kit_checked":"caller_side","bug_shaped":false}]}).to_string();
    validate_rule_map(fixture, &map, &[], |_| true).unwrap();
    let extra_exemption = format!("{fixture} `media.test.absent` is `not_observable`.");
    assert_eq!(
        validate_rule_map(&extra_exemption, &map, &[], |_| true).unwrap_err(),
        "exempt id not pinned: media.test.absent"
    );
    assert!(validate_rule_map(
        fixture,
        &map.replace("caller_side", "not_observable"),
        &[],
        |_| true
    )
    .is_err());
    assert!(validate_rule_map(
        fixture,
        &map.replace("media.test.rule", "media.test.info"),
        &[],
        |_| true
    )
    .is_err());
}

#[test]
fn media_kit_core_audio_fixtures() {
    let manifest = include_str!("../test-vectors/media-v1/SHA256SUMS");
    for (name, fixture, pinned) in [
        (
            "silence",
            AudioFixture::Silence,
            include_bytes!("../test-vectors/media-v1/audio/silence.wav").as_slice(),
        ),
        (
            "tone",
            AudioFixture::Tone,
            include_bytes!("../test-vectors/media-v1/audio/tone.wav").as_slice(),
        ),
        (
            "noise",
            AudioFixture::Noise,
            include_bytes!("../test-vectors/media-v1/audio/noise.wav").as_slice(),
        ),
    ] {
        assert_eq!(fixture.wav(), pinned, "fixture generator drift: {name}");
        assert!(manifest
            .lines()
            .any(|line| line == format!("{}  audio/{name}.wav", sha256(pinned))));
        assert_eq!(&pinned[..4], b"RIFF");
        assert_eq!(&pinned[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes(pinned[20..22].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(pinned[22..24].try_into().unwrap()), 1);
        assert_eq!(
            u32::from_le_bytes(pinned[24..28].try_into().unwrap()),
            16000
        );
        assert_eq!(u16::from_le_bytes(pinned[34..36].try_into().unwrap()), 16);
        assert_eq!(pinned.len() as u64, 44 + fixture.duration_ms() * 32);
    }
}

struct Empty;
#[async_trait]
impl MediaSubject for Empty {}

#[tokio::test]
async fn media_kit_core_default_hooks_fail_by_name() {
    let s = Empty;
    let principals = Principals::new("reader", vec![]);
    let failures = vec![
        ("open_watch", s.open_watch("alice", "call").await),
        (
            "poll_watch",
            s.poll_watch("alice", "call").await.map(|_| ()),
        ),
        ("configure_engines", s.configure_engines(vec![]).await),
        (
            "engine_declaration",
            s.engine_declaration().await.map(|_| ()),
        ),
        (
            "configure_peers",
            s.configure_peers(RecordingRoutingPeer::new(), RecordingVendor::new())
                .await,
        ),
        (
            "configure_principals",
            s.configure_principals(principals).await,
        ),
        (
            "call_as",
            s.call_as("alice", Request::new("media.status", json!({})))
                .await
                .map(|_| ()),
        ),
        (
            "configure_pack",
            s.configure_pack("agent", "bias", json!({})).await,
        ),
        (
            "script_engine",
            s.script_engine("model", Script::new(vec![])).await,
        ),
        ("engine_inputs", s.engine_inputs("model").await.map(|_| ())),
        ("hold_engine", s.hold_engine("model").await),
        ("release_engine", s.release_engine("model").await),
        ("restart", s.restart().await),
        ("advance_clock", s.advance_clock(1).await),
        ("seed_usage_entry", s.seed_usage_entry(vec![]).await),
    ];
    for (hook, result) in failures {
        let error = result.unwrap_err();
        assert_eq!(
            error.message,
            format!("subject hook {hook} is not implemented")
        );
        assert!(error.refusal.is_none());
    }
    assert!(
        matches!(run_case(&s,"empty_transcript_is_a_failure").await,Outcome::Failed(v) if v.contains("empty_transcript_is_a_failure") && v.contains("configure_peers"))
    );
    assert!(
        matches!(run_case(&s,"wire_extension").await,Outcome::NotApplicable(v) if v.contains("configure_engines"))
    );
}

#[tokio::test]
async fn media_kit_core_optional_capabilities() {
    for name in [
        "routing_admission_per_attempt",
        "refusal_class_decides_retry",
        "usage_record_per_attempt",
        "restart_keeps_terminal_jobs",
    ] {
        assert_eq!(
            run_case(&Fake::new(None, false, false), name).await,
            Outcome::Passed,
            "{name}"
        );
    }
    assert!(matches!(
        run_case(
            &Fake::new(None, false, false),
            "running_restart_records_failure"
        )
        .await,
        Outcome::NotApplicable(_)
    ));
}

struct Delayed {
    inner: Fake,
    polls: std::sync::atomic::AtomicUsize,
}
#[async_trait]
impl MediaSubject for Delayed {
    async fn configure_engines(
        &self,
        engines: Vec<cortexkit_role_media::describe::Engine>,
    ) -> SubjectResult<()> {
        self.inner.configure_engines(engines).await
    }
    async fn configure_peers(
        &self,
        routing: RecordingRoutingPeer,
        vendor: RecordingVendor,
    ) -> SubjectResult<()> {
        self.inner.configure_peers(routing, vendor).await
    }
    async fn configure_principals(&self, principals: Principals) -> SubjectResult<()> {
        self.inner.configure_principals(principals).await
    }
    async fn script_engine(&self, model: &str, script: Script) -> SubjectResult<()> {
        self.inner.script_engine(model, script).await
    }
    async fn engine_inputs(&self, model: &str) -> SubjectResult<Vec<Value>> {
        self.inner.engine_inputs(model).await
    }
    async fn call_as(&self, principal: &str, request: Request) -> SubjectResult<Value> {
        if request.method == "media.status" {
            let n = self
                .polls
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n < 2 {
                return Ok(json!({"result":{"status":if n==0 {"queued"} else {"running"}}}));
            }
        }
        self.inner.call_as(principal, request).await
    }
}

#[tokio::test]
async fn media_kit_core_polls_asynchronous_completion_without_advancing_clock() {
    let s = Delayed {
        inner: Fake::new(None, false, false),
        polls: std::sync::atomic::AtomicUsize::new(0),
    };
    assert_eq!(
        run_case(&s, "submit_idempotent_on_client_job_id").await,
        Outcome::Passed
    );
    assert!(s.polls.load(std::sync::atomic::Ordering::Relaxed) >= 3);
    // Delayed has no clock hook: polling would fail if it tried to advance time.
    assert!(s.advance_clock(0).await.is_err());
}
