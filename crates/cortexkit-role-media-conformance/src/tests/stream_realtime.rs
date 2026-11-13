use super::*;
use crate::checks::STREAM_REALTIME_CASES;

macro_rules! cases {
    ($($name:ident),* $(,)?) => {$ (
        #[tokio::test]
        async fn $name() {
            let case = stringify!($name).strip_prefix("media_kit_stream_realtime_").unwrap();
            assert_eq!(run_case(&Fake::new(None,true,true),case).await,Outcome::Passed,"conforming {case}");
            let bad = run_case(&Fake::new(Some(case),true,true),case).await;
            if case.starts_with("stream_audio_") {
                eprintln!("faulty {case}: {bad:?}");
            }
            assert!(matches!(bad,Outcome::Failed(ref reason) if reason.starts_with(&format!("{case}:"))),"faulty {case}: {bad:?}");
        }
    )*};
}
cases! {
    media_kit_stream_realtime_stream_input_limits,
    media_kit_stream_realtime_stream_audio_reorders_window,
    media_kit_stream_realtime_stream_audio_outside_window,
    media_kit_stream_realtime_stream_audio_principal_scope,
    media_kit_stream_realtime_stream_audio_crossing_frame,
    media_kit_stream_realtime_stream_id_reuse_refused,
    media_kit_stream_realtime_stream_first_frame_refused,
    media_kit_stream_realtime_stream_restart,
    media_kit_stream_realtime_stream_oversize_final_splits,
    media_kit_stream_realtime_stream_partials_replace_finals_append,
    media_kit_stream_realtime_stream_empty_result_needs_speech_check,
    media_kit_stream_realtime_stream_remote_requires_decision,
    media_kit_stream_realtime_stream_id_bounds,
    media_kit_stream_realtime_bias_trim_phrase_limit,
    media_kit_stream_realtime_context_pack_admission,
    media_kit_stream_realtime_context_pack_if_version,
    media_kit_stream_realtime_realtime_token_uses_requested_model,
    media_kit_stream_realtime_realtime_dictation_requires_realtime_engine,
    media_kit_stream_realtime_realtime_token_requires_decision,
    media_kit_stream_realtime_realtime_token_refuses_unadmitted_principal,
    media_kit_stream_realtime_realtime_token_bound_to_call,
    media_kit_stream_realtime_realtime_token_within_max_ttl,
    media_kit_stream_realtime_realtime_token_refuses_recorded_call,
    media_kit_stream_realtime_realtime_front_unavailable,
    media_kit_stream_realtime_realtime_front_check_order,
    media_kit_stream_realtime_realtime_front_dictation,
    media_kit_stream_realtime_call_started_replay_after_expiry,
    media_kit_stream_realtime_call_started_refuses_swapped_vendor_call,
    media_kit_stream_realtime_call_started_refuses_other_principal,
    media_kit_stream_realtime_calls_scoped_by_principal,
    media_kit_stream_realtime_call_ended_unrecorded,
    media_kit_stream_realtime_ended_call_never_reattaches,
    media_kit_stream_realtime_call_end_writes_one_record,
    media_kit_stream_realtime_call_id_bounds,
    media_kit_stream_realtime_stream_realtime_admission_lifecycle,
    media_kit_stream_realtime_stream_realtime_renewal_lapse,
}

#[tokio::test]
async fn media_kit_stream_realtime_watch_tool_shape_is_checked() {
    for fault in [
        "realtime_watch_tool_not_string",
        "realtime_watch_tool_outside_call",
    ] {
        let bad = run_case(&Fake::new(Some(fault), true, true), "realtime_watch").await;
        assert!(
            matches!(bad, Outcome::Failed(ref reason) if reason.starts_with("realtime_watch:")),
            "{bad:?}"
        );
    }
}

#[tokio::test]
async fn media_kit_stream_realtime_speech_check_capability() {
    let case = "stream_empty_result_needs_speech_check";
    assert_eq!(
        run_case(&Fake::new(None, true, true).with_speech_check(true), case).await,
        Outcome::Passed
    );
    assert!(
        matches!(run_case(&Fake::new(Some(case),true,true).with_speech_check(true),case).await,Outcome::Failed(reason) if reason.starts_with(case))
    );
}

#[tokio::test]
async fn media_kit_stream_realtime_admission_bound_independent_of_ttl() {
    let case = "realtime_token_within_max_ttl";
    let bad = run_case(
        &Fake::new(Some("realtime_token_admission_bound"), true, true),
        case,
    )
    .await;
    assert!(
        matches!(bad,Outcome::Failed(ref reason) if reason.starts_with(case) && reason.contains("admission validity")),
        "{bad:?}"
    );
}

#[test]
fn media_kit_stream_realtime_rule_map() {
    let map = validate_rule_map(CONTRACT, RULE_MAP, CASES, |id| {
        id.starts_with("media.stream.")
            || [
                "media.bias.trim",
                "media.packs.shape",
                "media.packs.admission",
                "media.realtime.token",
                "media.realtime.token_ref",
                "media.realtime.attach",
                "media.realtime.end",
            ]
            .contains(&id)
    })
    .unwrap();
    for case in STREAM_REALTIME_CASES {
        assert!(CASES.contains(case));
        assert!(
            map.rules.iter().any(|r| r.cases.iter().any(|c| c == case)),
            "unmapped case {case}"
        );
    }
    let r = map
        .rules
        .iter()
        .find(|r| r.id == "media.stream.answer")
        .unwrap();
    assert!(
        r.bug_shaped
            && r.cases
                .iter()
                .any(|c| c == "stream_empty_result_needs_speech_check")
    );
}

#[tokio::test]
#[allow(deprecated)]
async fn media_kit_stream_realtime_default_hooks_fail_by_name() {
    let empty = Empty;
    let errors = [
        (
            "configure_voice_front",
            empty
                .configure_voice_front("agent", None)
                .await
                .unwrap_err(),
        ),
        (
            "open_stream",
            empty.open_stream("alice", json!({})).await.unwrap_err(),
        ),
        (
            "send_stream_frame",
            empty
                .send_stream_frame("alice", "s", json!({}))
                .await
                .unwrap_err(),
        ),
        (
            "stream_audio_inputs",
            empty.stream_audio_inputs("alice", "s").await.unwrap_err(),
        ),
        (
            "poll_stream",
            empty.poll_stream("alice", "s").await.unwrap_err(),
        ),
        (
            "close_stream",
            empty.close_stream("alice", "s").await.unwrap_err(),
        ),
        (
            "refuse_next_stream",
            empty
                .refuse_next_stream("realtime_lane_full")
                .await
                .unwrap_err(),
        ),
    ];
    for (hook, e) in errors {
        assert!(e.message.contains(hook) && e.refusal.is_none());
    }
    let e = Fake::new(None, true, true)
        .refuse_next_stream("engine_crashed")
        .await
        .unwrap_err();
    assert!(e.message.contains("refuse_next_stream") && e.refusal.is_none());
}
