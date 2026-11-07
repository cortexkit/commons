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
        ("media.objects.read", "chunked_result_checksum"),
        ("media.output.nonempty", "empty_transcript_is_a_failure"),
        ("media.usage.decimal", "usage_no_rounding"),
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
        matches!(run_case(&s,"empty_transcript_is_a_failure").await,Outcome::Failed(v) if v.contains("empty_transcript_is_a_failure") && v.contains("configure_engines"))
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
                return Ok(json!({"status":if n==0 {"queued"} else {"running"}}));
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
