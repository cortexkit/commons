//! Tests of the runner itself, against the reference fake in `fake/`.
//! Conformance of a real provider is run by that provider, never here.

mod fake;

use cortexkit_role_tool_provider_conformance::{
    harness::KillMechanism, run_suite, Capability, CaseOutcome, SetupError, SuiteReport,
    SuiteVerdict, CASES,
};
use fake::{Defects, FakeSubject, LateCustodyDefect, SystemTextDefect, REFUSED_LISTED_PRESET};

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
    if case.starts_with("call_yield_") || case.starts_with("stop_session_") {
        if let Some(CaseOutcome::Inapplicable { reason }) = report.outcome(case) {
            assert!(reason.contains("adapter supplies no"), "{case}: {reason}");
            return;
        }
    }
    // The reference fake declares neither reply custody nor session notices,
    // so those cases are skipped or not applicable in every suite run against
    // it, whichever defect a test switches on.
    if case.starts_with("reply_custody_") {
        if let Some(CaseOutcome::Skipped { missing }) = report.outcome(case) {
            assert!(
                missing.contains(&Capability::ReplyCustody),
                "{case}: {missing:?}"
            );
            return;
        }
    }
    if case == "session_notices_describe" {
        if let Some(CaseOutcome::Inapplicable { .. }) = report.outcome(case) {
            return;
        }
    }
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
        if spec.name.starts_with("reply_custody_") {
            assert!(
                matches!(report.outcome(spec.name), Some(CaseOutcome::Skipped { missing })
                if missing.contains(&Capability::ReplyCustody))
            );
            continue;
        }
        if ["system_text_default_served", "session_notices_describe"].contains(&spec.name) {
            assert!(matches!(
                report.outcome(spec.name),
                Some(CaseOutcome::Inapplicable { .. })
            ));
            continue;
        }
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

const CARRIER_CUSTODY_CASES: &[&str] = &[
    "late_results_cursor_round_trip",
    "late_results_ack",
    "crash_after_prepared_not_started",
    "crash_after_authorized_not_started",
    "tool_result_held_reply_route_gating",
];

async fn rejects_late_custody_defect(defect: LateCustodyDefect, expected: &str) {
    let subject = FakeSubject::new(Defects {
        paginate_late_results: true,
        ..Defects::default()
    });
    let principals =
        cortexkit_role_tool_provider_conformance::ToolProviderSubject::scoped_principals(&subject)
            .unwrap();
    assert_ne!(principals.carrier, principals.scope.owner);
    let faithful = run(&subject).await;
    let failures: Vec<_> = CARRIER_CUSTODY_CASES
        .iter()
        .filter(|case| faithful.outcome(case) != Some(&CaseOutcome::Passed))
        .map(|case| format!("{case}: {:?}", faithful.outcome(case)))
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));

    let faulty = run(&FakeSubject::new(Defects {
        late_custody: Some(defect),
        paginate_late_results: true,
        ..Defects::default()
    }))
    .await;
    for case in CARRIER_CUSTODY_CASES {
        let reason = failed(&faulty, case);
        assert!(reason.contains(expected), "{case}: {reason}");
        assert!(reason.contains("carrier"), "{case}: {reason}");
        assert!(reason.contains("owner"), "{case}: {reason}");
        println!("rejected {case}: {reason}");
    }
}

#[tokio::test]
async fn carrier_custody_rejects_owner_delivery() {
    rejects_late_custody_defect(LateCustodyDefect::OwnerInsteadOfCarrier, "scope owner").await;
}

#[tokio::test]
async fn carrier_custody_rejects_owner_feed_leakage() {
    rejects_late_custody_defect(LateCustodyDefect::CopyToOwner, "scope owner").await;
}

#[tokio::test]
async fn carrier_custody_rejects_duplicate_entries() {
    rejects_late_custody_defect(LateCustodyDefect::DuplicateCarrierEntry, "found 2").await;
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
async fn an_untyped_property_fails_the_portability_case_by_tool_and_path() {
    let report = run(&FakeSubject::new(Defects {
        untyped_property: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_schemas_portable");
    assert!(reason.contains(fake::SLOW), "{reason}");
    assert!(reason.contains("/properties/surface"), "{reason}");
    assert!(reason.contains("declares no type"), "{reason}");
    // The schema is valid JSON Schema, so only the portability case objects.
    assert_passed(&report, "catalog_schemas_flat");
    assert_passed(&report, "catalog_schema_digest_stable");
    assert_passed(&report, "catalog_reply_valid");
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
async fn malformed_tool_result_v1_reply_fails_by_case_tool_and_member() {
    let report = run(&FakeSubject::new(Defects {
        malformed_result: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "tool_result_v1_valid");
    assert!(reason.contains(fake::QUICK), "{reason}");
    assert!(
        reason.contains("content: must contain at least one part"),
        "{reason}"
    );
    assert_passed(&report, "catalog_schemas_flat");
    assert_passed(&report, "terminal_frame_on_success");
}

#[tokio::test]
async fn tool_result_v1_is_sent_only_to_a_route_that_declares_support() {
    let mut subject = FakeSubject::new(Defects {
        ignore_consumer_capabilities: true,
        mcp_shaped_legacy: true,
        ..Defects::default()
    });
    subject.result_calls = vec![(fake::QUICK.into(), serde_json::json!({"fail": true}))];
    let report = run(&subject).await;
    let reason = failed(&report, "tool_result_legacy_route_unchanged");
    assert!(reason.contains(fake::QUICK), "{reason}");
    assert!(reason.contains("without consumer declaration"), "{reason}");
    assert!(reason.contains("JSON pointer \"/errorCode\""), "{reason}");
    assert_passed(&report, "tool_result_v1_valid");
    assert_passed(&report, "terminal_frame_on_success");
}

#[tokio::test]
async fn mcp_shaped_legacy_reply_matches_the_supplied_fixture() {
    let mut subject = FakeSubject::new(Defects {
        mcp_shaped_legacy: true,
        ..Defects::default()
    });
    subject.result_calls = subject
        .legacy_replies
        .iter()
        .map(|(call, _)| call.clone())
        .collect();
    for (_, body) in &subject.legacy_replies {
        assert!(
            serde_json::from_value::<cortexkit_role_tool_provider::result::ToolResult>(
                body.clone()
            )
            .is_ok()
        );
    }
    let report = run(&subject).await;
    assert_passed(&report, "tool_result_legacy_route_unchanged");
    assert_passed(&report, "tool_result_v1_valid");
}

#[tokio::test]
async fn without_legacy_reply_the_legacy_case_is_skipped_never_passed() {
    let mut subject = FakeSubject::new(Defects {
        mcp_shaped_legacy: true,
        ..Defects::default()
    });
    subject.legacy_replies.clear();
    subject.capabilities.remove(&Capability::LegacyReply);
    // Skip the simulated crash_after_prepared_not_started and
    // crash_after_authorized_not_started cases: they fail the suite independently
    // of legacy-reply coverage because they cannot kill a real process.
    subject.capabilities.remove(&Capability::ApprovalExecution);
    let report = run(&subject).await;
    assert_passed(&report, "tool_result_v1_valid");
    assert_eq!(
        report.outcome("tool_result_legacy_route_unchanged"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::LegacyReply],
        })
    );
    assert_eq!(
        report.verdict,
        SuiteVerdict::ConformingForDeclaredCapabilities {
            skipped: vec![
                Capability::ApprovalExecution,
                Capability::ReplyCustody,
                Capability::ReplyCustodyPure,
                Capability::ReplyCustodyHeld,
                Capability::ReplyCustodyRefusal,
                Capability::ReplyCustodyCatalogChange,
                Capability::LegacyReply
            ],
        },
        "{}",
        report.render()
    );
    assert!(report.render().contains(
        "SKIP tool_result_legacy_route_unchanged (requires tool-result/v1, legacy_reply; missing legacy_reply)"
    ));
}

#[tokio::test]
async fn declaring_legacy_reply_without_a_fixture_fails_by_tool_name() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.legacy_replies.clear();
    let report = run(&subject).await;
    let reason = failed(&report, "tool_result_legacy_route_unchanged");
    assert!(reason.contains(fake::QUICK), "{reason}");
    assert!(
        reason.contains("legacy_reply missing for probe"),
        "{reason}"
    );
    assert_passed(&report, "tool_result_v1_valid");
}

#[tokio::test]
async fn legacy_reply_mismatch_names_the_first_differing_json_pointer() {
    use serde_json::json;
    for (expected, pointer) in [
        (
            json!({"content": [{"type": "text", "text": "Changed."}], "isError": false}),
            "/content/0/text",
        ),
        (json!({"content": [], "isError": false}), "/content/0"),
        (
            json!({"content": [{"type": "text", "text": "Legacy greeting."}, null], "isError": false}),
            "/content/1",
        ),
        (
            json!({"content": [{"type": "text", "text": "Legacy greeting."}], "isError": false, "a~/": null}),
            "/a~0~1",
        ),
        (
            json!({"content": [{"type": "text", "text": "Legacy greeting."}]}),
            "/isError",
        ),
        (json!(null), ""),
    ] {
        let mut subject = FakeSubject::new(Defects {
            mcp_shaped_legacy: true,
            ..Defects::default()
        });
        subject.legacy_replies[0].1 = expected;
        let report = run(&subject).await;
        let reason = failed(&report, "tool_result_legacy_route_unchanged");
        assert!(reason.contains(fake::QUICK), "{reason}");
        assert!(
            reason.ends_with(&format!("JSON pointer {pointer:?}")),
            "{reason}"
        );
    }
}

#[tokio::test]
async fn held_reply_requires_both_route_declarations() {
    let report = run(&FakeSubject::new(Defects {
        held_on_content_only: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "tool_result_held_reply_route_gating");
    assert!(reason.contains("held marker disagrees"), "{reason}");
    assert!(reason.contains("[\"tool-result/v1\"]"), "{reason}");
    assert_passed(&report, "tool_result_v1_valid");
}

#[tokio::test]
async fn held_reply_case_requires_a_positive_held_marker() {
    let report = run(&FakeSubject::new(Defects {
        omit_held_marker: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "tool_result_held_reply_route_gating");
    assert!(reason.contains("held marker disagrees"), "{reason}");
    assert_passed(&report, "tool_result_v1_valid");
}

#[tokio::test]
async fn held_reply_case_validates_the_interim_body() {
    let report = run(&FakeSubject::new(Defects {
        held_is_error: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "tool_result_held_reply_route_gating");
    assert!(
        reason.contains("held: must not accompany isError: true"),
        "{reason}"
    );
    assert_passed(&report, "tool_result_v1_valid");
}

#[tokio::test]
async fn undeclared_held_reply_fixture_is_skipped_by_name() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::HeldReplyV1);
    let report = run(&subject).await;
    assert_eq!(
        report.outcome("tool_result_held_reply_route_gating"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::HeldReplyV1],
        })
    );
}

#[tokio::test]
async fn opaque_tools_skip_tool_result_v1_by_name_never_pass() {
    let mut subject = FakeSubject::new(Defects {
        opaque_result: true,
        ..Defects::default()
    });
    // Even a manually supplied flag cannot substitute for the catalog declaration.
    subject.capabilities.insert(Capability::ToolResultV1);
    let report = run(&subject).await;
    assert_eq!(
        report.outcome("tool_result_v1_valid"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::ToolResultV1],
        })
    );
    assert!(report.render().contains("SKIP tool_result_v1_valid"));
    assert_passed(&report, "terminal_frame_on_success");
}

#[tokio::test]
async fn catalog_tool_result_v1_declaration_cannot_be_opted_out_of() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.remove(&Capability::ToolResultV1);
    let report = run(&subject).await;
    assert_passed(&report, "tool_result_v1_valid");
}

#[tokio::test]
async fn invalid_modes_fail_the_catalog_case_by_tool_and_member() {
    let report = run(&FakeSubject::new(Defects {
        invalid_modes: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_modes_valid");
    assert!(reason.contains("invalid_answer: echo modes:"), "{reason}");
    assert!(reason.contains("must not be empty"), "{reason}");
}

#[tokio::test]
async fn a_null_emits_events_fails_the_boolean_case_by_tool_and_member() {
    // `null` decodes as absence, so only a check of the raw answer sees it;
    // every other catalog case still passes.
    let report = run(&FakeSubject::new(Defects {
        emits_events_override: Some("null"),
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_emits_events_boolean");
    assert!(
        reason.contains("invalid_answer: echo emits_events: must be a boolean, got null"),
        "{reason}"
    );
    assert_passed(&report, "catalog_schemas_flat");
    assert_passed(&report, "catalog_modes_valid");
}

#[tokio::test]
async fn a_string_emits_events_fails_the_boolean_case_by_tool_and_member() {
    let report = run(&FakeSubject::new(Defects {
        emits_events_override: Some("\"false\""),
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_emits_events_boolean");
    assert!(
        reason.contains("invalid_answer: echo emits_events: must be a boolean, got \"false\""),
        "{reason}"
    );
}

#[tokio::test]
async fn ignoring_bind_mode_fails_the_mode_case() {
    let report = run(&FakeSubject::new(Defects {
        ignore_call_mode: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "call_mode_matches_catalog");
    assert!(reason.contains("an unlisted-mode call"), "{reason}");
    for case in &report.cases {
        if case.case == "system_text_default_served" {
            assert!(matches!(case.outcome, CaseOutcome::Inapplicable { .. }));
        } else if case.case != "call_mode_matches_catalog" {
            assert_passed(&report, case.case);
        }
    }
}

#[tokio::test]
async fn refusing_a_listed_mode_fails_the_mode_case() {
    let report = run(&FakeSubject::new(Defects {
        refuse_served_mode: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "call_mode_matches_catalog");
    assert!(reason.contains("the listed-mode call failed"), "{reason}");
    assert_passed(&report, "catalog_modes_valid");
    assert_passed(&report, "terminal_frame_on_success");
}

#[tokio::test]
async fn no_restricted_mode_fixture_is_skipped_by_name_never_passed() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.mode_probe = None;
    let report = run(&subject).await;
    assert_eq!(
        report.outcome("call_mode_matches_catalog"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::CallModes],
        })
    );
    assert!(report
        .render()
        .contains("SKIP call_mode_matches_catalog (requires call_modes; missing call_modes)"));
    assert!(!report.render().contains("PASS call_mode_matches_catalog"));
    assert_passed(&report, "catalog_modes_valid");
}

#[tokio::test]
async fn catalog_capability_filter_is_skipped_when_role_describe_does_not_advertise_it() {
    let report = run(&FakeSubject::new(Defects {
        omit_catalog_capability_filter: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.outcome("catalog_capability_filter_matches_describe"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::CatalogCapabilityFilter],
        })
    );
    assert!(report
        .render()
        .contains("SKIP catalog_capability_filter_matches_describe"));
    assert!(!report
        .render()
        .contains("PASS catalog_capability_filter_matches_describe"));
    for case in [
        "catalog_filter_preserves_plan_item",
        "catalog_filter_model_refused",
    ] {
        assert_eq!(
            report.outcome(case),
            Some(&CaseOutcome::Skipped {
                missing: vec![Capability::CatalogCapabilityFilter],
            })
        );
    }
    assert_passed(&report, "catalog_model_malformed_refused");
}

#[tokio::test]
async fn ordinary_plan_item_with_documented_params_passes_the_filter_case() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_passed(&report, "catalog_filter_preserves_plan_item");
}

#[tokio::test]
async fn empty_ordinary_catalog_fails_the_plan_item_case() {
    let report = run(&FakeSubject::new(Defects {
        empty_ordinary_catalog: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_filter_preserves_plan_item");
    assert!(reason.contains("non-empty catalog"), "{reason}");
}

#[tokio::test]
async fn refusing_documented_params_fails_the_plan_item_case() {
    let report = run(&FakeSubject::new(Defects {
        refuse_ordinary_scope: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_filter_preserves_plan_item");
    assert!(reason.contains("params.scope"), "{reason}");
}

#[tokio::test]
async fn accepting_malformed_model_fails_the_model_case() {
    let report = run(&FakeSubject::new(Defects {
        accept_malformed_model: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_model_malformed_refused");
    assert!(reason.contains("not an error frame"), "{reason}");
    assert_passed(&report, "catalog_filter_preserves_plan_item");
}

#[tokio::test]
async fn accepting_model_on_filter_fails_the_filter_model_case() {
    let report = run(&FakeSubject::new(Defects {
        accept_filter_model: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_filter_model_refused");
    assert!(reason.contains("not an error frame"), "{reason}");
    assert_passed(&report, "catalog_model_malformed_refused");
}

#[tokio::test]
async fn catalog_model_case_is_skipped_without_the_live_token() {
    let report = run(&FakeSubject::new(Defects {
        omit_catalog_model: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.outcome("catalog_model_malformed_refused"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::CatalogModel],
        })
    );
    assert_passed(&report, "catalog_filter_model_refused");
    assert_passed(&report, "catalog_filter_preserves_plan_item");
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
async fn refusing_a_listed_preset_fails_the_listed_presets_case_by_name() {
    let report = run(&FakeSubject::new(Defects {
        list_refused_preset: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "catalog_listed_presets_served");
    assert!(
        reason.contains(&format!("{REFUSED_LISTED_PRESET:?}")),
        "{reason}"
    );
    assert!(reason.contains("not a response"), "{reason}");
    assert_passed(&report, "catalog_unknown_preset_refused");
    assert_passed(&report, "catalog_digest_only");
}

#[tokio::test]
async fn a_provider_listing_no_presets_skips_the_listed_presets_case() {
    let report = run(&FakeSubject::new(Defects {
        list_no_presets: true,
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.outcome("catalog_listed_presets_served"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::Presets],
        })
    );
    assert!(report
        .render()
        .contains("SKIP catalog_listed_presets_served (requires presets; missing presets)"));
    assert_passed(&report, "catalog_unknown_preset_refused");
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
        // The optional-preset case applies the same checks to its preset-less
        // answer, so the wrapped digest fails it too, under its own name.
        if spec.name == "system_text_preset_optional_served" {
            let reason = failed(&report, spec.name);
            assert!(reason.contains("system_text.item_digest"), "{reason}");
        } else if !spec.name.starts_with("reply_custody_")
            && ![
                "system_text_digests_match_text",
                "system_text_default_served",
                "session_notices_describe",
            ]
            .contains(&spec.name)
        {
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
async fn refusing_a_preset_less_system_text_item_fails_the_optional_preset_case_by_name() {
    // A provider whose release still requires the item's preset refuses the
    // request as malformed; the other system-text cases, which always send a
    // preset, still pass.
    let report = run(&FakeSubject::new(Defects {
        refuse_preset_less_system_text: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "system_text_preset_optional_served");
    assert!(reason.contains("refused as invalid_params"), "{reason}");
    assert!(reason.contains("without a preset"), "{reason}");
    for spec in CASES {
        if !spec.name.starts_with("reply_custody_")
            && ![
                "system_text_preset_optional_served",
                "system_text_default_served",
                "session_notices_describe",
            ]
            .contains(&spec.name)
        {
            assert_passed(&report, spec.name);
        }
    }
}

#[tokio::test]
async fn answering_another_variant_without_a_preset_fails_the_optional_preset_case_by_name() {
    let report = run(&FakeSubject::new(Defects {
        preset_less_system_text_is_lean: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "system_text_preset_optional_served");
    assert!(reason.contains("system_text.text received"), "{reason}");
    assert!(
        reason.contains("the answer for the default preset \"default\""),
        "{reason}"
    );
    assert_passed(&report, "system_text_digests_match_text");
    assert_passed(&report, "system_text_preflight_digest_stable");
}

#[tokio::test]
async fn an_unnamed_default_preset_checks_only_that_the_preset_less_item_is_served() {
    // Without a named default the kit cannot tell which variant answered, so
    // a wrong variant passes; a refusal still fails.
    let mut subject = FakeSubject::new(Defects {
        preset_less_system_text_is_lean: true,
        ..Defects::default()
    });
    subject.system_text_default_preset = None;
    assert_passed(&run(&subject).await, "system_text_preset_optional_served");

    let mut subject = FakeSubject::new(Defects {
        refuse_preset_less_system_text: true,
        ..Defects::default()
    });
    subject.system_text_default_preset = None;
    let report = run(&subject).await;
    let reason = failed(&report, "system_text_preset_optional_served");
    assert!(reason.contains("refused as invalid_params"), "{reason}");
}

#[tokio::test]
async fn a_provider_not_declaring_the_optional_preset_skips_the_case_by_name() {
    // An older provider refuses a preset-less item; not declaring the
    // capability reports the case as not applicable instead of failed.
    let mut subject = FakeSubject::new(Defects {
        refuse_preset_less_system_text: true,
        ..Defects::default()
    });
    subject
        .capabilities
        .remove(&Capability::SystemTextOptionalPreset);
    let report = run(&subject).await;
    assert_eq!(
        report.outcome("system_text_preset_optional_served"),
        Some(&CaseOutcome::Skipped {
            missing: vec![Capability::SystemTextOptionalPreset],
        })
    );
    assert!(report.render().contains(
        "SKIP system_text_preset_optional_served (requires system_text, \
         system_text_optional_preset; missing system_text_optional_preset)"
    ));
    assert_passed(&report, "system_text_digests_match_text");
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
        "system_text_preset_optional_served",
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
                Capability::ReplyCustody,
                Capability::ReplyCustodyPure,
                Capability::ReplyCustodyHeld,
                Capability::ReplyCustodyRefusal,
                Capability::ReplyCustodyCatalogChange,
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

#[tokio::test]
async fn user_command_faults_fail_their_named_cases() {
    for (defect, case) in [
        ("catalog", "user_commands_catalog"),
        ("reply", "user_commands_reply"),
        ("repeat", "user_commands_repeat"),
        ("repeat_bytes", "user_commands_repeat"),
        ("codemode", "user_commands_codemode_refused"),
        ("flow", "user_commands_flow_refused"),
        ("accepted", "user_commands_accepted_late"),
    ] {
        let report = run(&FakeSubject::new(Defects {
            user_command: Some(defect),
            ..Defects::default()
        }))
        .await;
        failed(&report, case);
    }
}

#[tokio::test]
async fn user_command_codemode_scope_is_refused() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert_passed(&report, "user_commands_codemode_refused");
    assert_passed(&report, "user_commands_flow_refused");
}

#[tokio::test]
async fn system_text_default_served() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.insert(Capability::SystemTextDefault);
    assert_passed(&run(&subject).await, "system_text_default_served");
}

#[tokio::test]
async fn system_text_default_refusing_provider_fails_by_name() {
    let mut subject = FakeSubject::new(Defects {
        refuse_preset_less_system_text: true,
        ..Defects::default()
    });
    subject.capabilities.insert(Capability::SystemTextDefault);
    subject
        .capabilities
        .remove(&Capability::SystemTextOptionalPreset);
    let report = run(&subject).await;
    let reason = failed(&report, "system_text_default_served");
    assert!(reason.contains("refused as invalid_params"), "{reason}");
    let failures: Vec<_> = report
        .cases
        .iter()
        .filter_map(|case| matches!(case.outcome, CaseOutcome::Failed { .. }).then_some(case.case))
        .collect();
    assert_eq!(failures, ["system_text_default_served"]);
}

#[tokio::test]
async fn system_text_default_undeclared_is_not_applicable() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    assert!(matches!(report.outcome("system_text_default_served"),
        Some(CaseOutcome::Inapplicable { reason }) if reason == "provider does not declare system-text-default/v1"));
    assert!(report.render().contains("N/A system_text_default_served"));
}

#[tokio::test]
async fn reply_custody_undeclared_cases_are_skipped_never_passed() {
    let report = run(&FakeSubject::new(Defects::default())).await;
    let mut count = 0;
    for spec in CASES
        .iter()
        .filter(|spec| spec.name.starts_with("reply_custody_"))
    {
        count += 1;
        assert!(
            matches!(report.outcome(spec.name), Some(CaseOutcome::Skipped { missing })
            if missing.contains(&Capability::ReplyCustody)),
            "{}: {:?}",
            spec.name,
            report.outcome(spec.name)
        );
    }
    assert_eq!(count, 12);
}

#[tokio::test]
async fn reply_custody_declared_missing_fixtures_fail_by_case_name() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.capabilities.extend([
        Capability::ReplyCustody,
        Capability::ReplyCustodyPure,
        Capability::ReplyCustodyHeld,
        Capability::ReplyCustodyRefusal,
        Capability::ReplyCustodyCatalogChange,
    ]);
    let report = run(&subject).await;
    for spec in CASES
        .iter()
        .filter(|spec| spec.name.starts_with("reply_custody_"))
    {
        let reason = failed(&report, spec.name);
        assert!(
            reason.contains("no reply-custody lifecycle fixture supplied"),
            "{}: {reason}",
            spec.name
        );
    }
}

#[tokio::test]
async fn session_notices_live_token_enables_describe_case() {
    let subject = FakeSubject::new(Defects {
        session_notices_token: Some("session-notices/v1"),
        ..Defects::default()
    });
    assert!(!subject.capabilities.contains(&Capability::SessionNotices));
    assert_passed(&run(&subject).await, "session_notices_describe");
}

#[tokio::test]
async fn session_notices_malformed_capabilities_fail_the_named_case() {
    let report = run(&FakeSubject::new(Defects {
        session_notices_token: Some("session-notices/v1"),
        malformed_notice_capabilities: true,
        ..Defects::default()
    }))
    .await;
    assert!(failed(&report, "session_notices_describe").contains("Undecodable"));
}

#[tokio::test]
async fn session_notices_unknown_or_absent_token_is_not_applicable() {
    for token in [None, Some("session-notices/future")] {
        let report = run(&FakeSubject::new(Defects {
            session_notices_token: token,
            ..Defects::default()
        }))
        .await;
        assert!(matches!(
            report.outcome("session_notices_describe"),
            Some(CaseOutcome::Inapplicable { .. })
        ));
    }
}
