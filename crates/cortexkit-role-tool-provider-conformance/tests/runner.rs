//! Tests of the runner itself, against the reference fake in `fake/`.
//! Conformance of a real provider is run by that provider, never here.

mod fake;

use cortexkit_role_tool_provider_conformance::{
    harness::KillMechanism, run_suite, Capability, CaseOutcome, SetupError, SuiteReport,
    SuiteVerdict, CASES,
};
use fake::{Defects, FakeSubject, SystemTextDefect};

async fn run(subject: &FakeSubject) -> SuiteReport {
    let dir = tempfile::tempdir().unwrap();
    run_suite(subject, dir.path()).await.unwrap()
}

fn failed(report: &SuiteReport, case: &str) -> String {
    match report.outcome(case) {
        Some(CaseOutcome::Failed { reason }) => reason.clone(),
        other => panic!("{case} should fail, got {other:?}\n{}", report.render()),
    }
}

fn assert_passed(report: &SuiteReport, case: &str) {
    assert_eq!(
        report.outcome(case),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn a_faithful_provider_passes_every_case_but_its_simulated_kills_fail_the_run() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_eq!(report.cases.len(), CASES.len());
    for spec in CASES {
        assert_passed(&report, spec.name);
    }
    assert_eq!(report.kills.len(), 2);
    assert!(report
        .kills
        .iter()
        .all(|kill| kill.mechanism == KillMechanism::FaultHook));
    match &report.verdict {
        SuiteVerdict::Failed { reasons } => {
            assert_eq!(reasons.len(), 1, "{reasons:?}");
            assert!(reasons[0].contains("no kill in this run ended a real process"));
        }
        other => panic!("expected the real-kill rule to fail the run, got {other:?}"),
    }
}

#[tokio::test]
async fn a_second_terminal_frame_on_cancel_fails_the_terminal_frame_case() {
    let report = run(&FakeSubject::new(Defects {
        double_terminal_on_cancel: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "terminal_frame_on_cancel");
    assert!(reason.contains("2 terminal frames"), "{reason}");
    assert_passed(&report, "terminal_frame_on_success");
}

#[tokio::test]
async fn refusing_a_key_never_held_fails_the_unknown_call_case() {
    let report = run(&FakeSubject::new(Defects {
        refuse_unknown_call: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "withdraw_unknown_call");
    assert!(reason.contains("not unknown_call"), "{reason}");
}

#[tokio::test]
async fn running_a_held_call_after_restart_fails_both_crash_cases() {
    let report = run(&FakeSubject::new(Defects {
        run_held_calls_on_restart: true,
        ..Defects::default()
    }))
    .await;
    for case in [
        "crash_after_prepared_not_started",
        "crash_after_authorized_not_started",
    ] {
        let reason = failed(&report, case);
        assert!(reason.contains("ran the call"), "{case}: {reason}");
    }
}

#[tokio::test]
async fn a_root_level_union_fails_the_flatness_case() {
    let report = run(&FakeSubject::new(Defects {
        root_union_schema: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_schemas_flat");
    assert!(reason.contains("anyOf"), "{reason}");
}

#[tokio::test]
async fn an_invalid_reply_fails_the_catalog_reply_case_by_tool_and_member() {
    let report = run(&FakeSubject::new(Defects {
        invalid_reply: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_reply_valid");
    assert!(reason.contains("invalid_answer"), "{reason}");
    assert!(reason.contains(fake::QUICK), "{reason}");
    assert!(reason.contains("reply.max_ms"), "{reason}");
    assert_passed(&report, "catalog_schemas_flat");
    assert_passed(&report, "catalog_schema_digest_stable");
    assert_passed(&report, "catalog_digest_only");
}

#[tokio::test]
async fn an_undefined_unprefixed_capability_tag_fails_the_catalog_case() {
    let report = run(&FakeSubject::new(Defects {
        undefined_unprefixed_tag: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_schemas_flat");
    assert!(reason.contains("code.refactor/v1"), "{reason}");
    assert!(reason.contains("DEFINED_CAPABILITY_TAGS"), "{reason}");
}

#[tokio::test]
async fn guessing_a_variant_for_an_undefined_preset_fails_the_unknown_preset_case() {
    let report = run(&FakeSubject::new(Defects {
        accept_any_preset: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_unknown_preset_refused");
    assert!(reason.contains("not an error frame"), "{reason}");
    assert_passed(&report, "catalog_digest_only");
}

#[tokio::test]
async fn digesting_a_json_wrapper_fails_system_text_digests_match_text_by_name() {
    let report = run(&FakeSubject::new(Defects {
        system_text: Some(SystemTextDefect::WrappedDigests),
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "system_text_digests_match_text");
    let expected = cortexkit_role_tool_provider_conformance::wire::catalog::system_text_digest(
        fake::SYSTEM_TEXT,
    );
    let received = cortexkit_role_tool_provider_conformance::wire::catalog::composition_digest(
        &serde_json::json!({ "text": fake::SYSTEM_TEXT }),
    )
    .unwrap();
    assert_ne!(received, expected);
    assert!(reason.contains("system_text.item_digest"), "{reason}");
    assert!(reason.contains(&received), "{reason}");
    assert!(
        reason.contains(&format!("expected digest {expected}")),
        "{reason}"
    );
    for spec in CASES {
        if spec.name != "system_text_digests_match_text" {
            assert_passed(&report, spec.name);
        }
    }
}

#[tokio::test]
async fn item_digest_hashes_text_and_both_digests_use_lowercase_hex() {
    use SystemTextDefect::*;
    for (defect, field) in [
        (WrappedItemDigest, "item_digest"),
        (UppercaseItemDigest, "item_digest"),
        (UppercasePreflightDigest, "preflight_digest"),
        (MissingItemDigest, "item_digest"),
        (MissingPreflightDigest, "preflight_digest"),
    ] {
        let report = run(&FakeSubject::new(Defects {
            system_text: Some(defect),
            ..Defects::default()
        }))
        .await;
        let reason = failed(&report, "system_text_digests_match_text");
        assert!(
            reason.contains(&format!("system_text.{field} received")),
            "{defect:?}: {reason}"
        );
        let expected = cortexkit_role_tool_provider_conformance::wire::catalog::system_text_digest(
            fake::SYSTEM_TEXT,
        );
        if field == "item_digest" {
            assert!(
                reason.contains(&format!("expected digest {expected}")),
                "{defect:?}: {reason}"
            );
        } else {
            assert!(reason.contains("provider-defined"), "{defect:?}: {reason}");
            assert!(
                reason.contains("64 lowercase hex characters"),
                "{defect:?}: {reason}"
            );
        }
        assert_passed(&report, "catalog_digest_only");
    }
}

#[tokio::test]
async fn system_text_requires_text_and_sorted_deduplicated_tool_names() {
    use SystemTextDefect::*;
    for (defect, field) in [
        (MissingAnswer, "system_text received"),
        (MissingText, "system_text.text received"),
        (MissingToolNames, "system_text.tool_names received"),
        (UnsortedToolNames, "system_text.tool_names received"),
        (DuplicateToolNames, "system_text.tool_names received"),
    ] {
        let report = run(&FakeSubject::new(Defects {
            system_text: Some(defect),
            ..Defects::default()
        }))
        .await;
        let reason = failed(&report, "system_text_digests_match_text");
        assert!(reason.contains(field), "{defect:?}: {reason}");
        assert_passed(&report, "catalog_digest_only");
    }
}

#[tokio::test]
async fn declaring_system_text_without_request_arguments_fails_the_case() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.system_text_arguments = None;
    let report = run(&subject).await;
    let reason = failed(&report, "system_text_digests_match_text");
    assert!(
        reason.contains("supplies no system-text catalog arguments"),
        "{reason}"
    );
    subject.system_text_arguments = Some(serde_json::json!({ "params": {} }));
    let report = run(&subject).await;
    let reason = failed(&report, "system_text_digests_match_text");
    assert!(
        reason.contains("request system_text received null"),
        "{reason}"
    );
    subject.system_text_arguments = Some(serde_json::json!({
        "system_text": { "preset": "default", "params": {} }, "digest_only": true,
    }));
    let report = run(&subject).await;
    let reason = failed(&report, "system_text_digests_match_text");
    assert!(
        reason.contains("digest_only, not a full answer"),
        "{reason}"
    );
}

#[tokio::test]
async fn a_provider_defined_preflight_digest_need_not_equal_the_text_digest() {
    let text_digest = cortexkit_role_tool_provider_conformance::wire::catalog::system_text_digest(
        fake::SYSTEM_TEXT,
    );
    assert_ne!(fake::system_text_preflight_digest(), text_digest);
    for defect in [None, Some(SystemTextDefect::WrappedPreflightDigest)] {
        let report = run(&FakeSubject::new(Defects {
            system_text: defect,
            ..Defects::default()
        }))
        .await;
        assert_passed(&report, "system_text_digests_match_text");
        assert_passed(&report, "system_text_preflight_digest_stable");
    }
}

#[tokio::test]
async fn changing_either_digest_fails_system_text_preflight_digest_stable_by_name() {
    for (defect, field) in [
        (SystemTextDefect::ChangingItemDigest, "item_digest"),
        (
            SystemTextDefect::ChangingPreflightDigest,
            "preflight_digest",
        ),
    ] {
        let report = run(&FakeSubject::new(Defects {
            system_text: Some(defect),
            ..Defects::default()
        }))
        .await;
        let reason = failed(&report, "system_text_preflight_digest_stable");
        assert!(
            reason.contains(&format!("second system_text.{field} received")),
            "{defect:?}: {reason}"
        );
        assert!(reason.contains("expected digest"), "{reason}");
        assert_passed(&report, "system_text_digests_match_text");
    }
}

#[tokio::test]
async fn a_case_whose_capability_is_undeclared_is_skipped_never_passed() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::ApprovalExecution);
    subject.capabilities.remove(&Capability::DisableTool);
    subject.capabilities.remove(&Capability::SystemText);
    let report = run(&subject).await;
    for case in [
        "catalog_disabled_tool_absent",
        "call_disabled_tool_refused_by_name",
        "system_text_digests_match_text",
        "system_text_preflight_digest_stable",
        "late_results_cursor_round_trip",
        "late_results_ack",
        "crash_after_prepared_not_started",
        "crash_after_authorized_not_started",
    ] {
        assert!(
            matches!(report.outcome(case), Some(CaseOutcome::Skipped { missing }) if !missing.is_empty()),
            "{case}: {:?}",
            report.outcome(case)
        );
    }
    assert!(report.kills.is_empty());
    assert_eq!(
        report.verdict,
        SuiteVerdict::ConformingForDeclaredCapabilities {
            skipped: vec![
                Capability::DisableTool,
                Capability::ApprovalExecution,
                Capability::SystemText
            ],
        },
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn forgetting_acks_fails_the_ack_case() {
    let report = run(&FakeSubject::new(Defects {
        forget_acks: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "late_results_ack");
    assert!(reason.contains("still served"), "{reason}");
    assert_passed(&report, "late_results_cursor_round_trip");
}

#[tokio::test]
async fn a_used_work_dir_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("left-over"), b"x").unwrap();
    let error = run_suite(&FakeSubject::new(Defects::default()), dir.path())
        .await
        .unwrap_err();
    assert!(matches!(error, SetupError::WorkDirNotEmpty(_)));
}
