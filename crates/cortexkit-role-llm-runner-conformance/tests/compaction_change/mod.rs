use super::*;
use cortexkit_role_llm_runner_conformance::{LlmRunnerSubject, RefreshVariant};

const NAMES: &[&str] = &[
    "compaction_add_takes_effect_next_run",
    "compaction_remove_is_refused",
    "compaction_switch_accept_or_refuse",
];

fn subject(fault: Option<&'static str>) -> FakeSubject {
    let mut subject = changing(Defects {
        compaction_change_fixture: true,
        compaction_change: fault,
        ..Defects::default()
    });
    subject.capabilities.insert(Capability::Plans);
    subject
}

#[tokio::test]
async fn compaction_changes_accepting_switch_pass_all_three_cases() {
    let subject = subject(None);
    assert!(subject.send_fields("compaction_switch_probe", true)["plan"]
        .get("compaction_item")
        .is_none());
    assert!(!subject
        .refresh_plan("compaction_switch_probe", RefreshVariant::ChangedTools)
        .unwrap()
        .plan
        .contains_key("compaction_item"));
    let report = run(&subject).await;
    for name in NAMES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn compaction_changes_refusing_switch_pass_all_three_cases() {
    let report = run(&subject(Some("refuse_switch"))).await;
    for name in NAMES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn compaction_changes_send_fields_fallback_passes_all_three_cases() {
    let mut subject = subject(None);
    subject.defects.compaction_change_fixture = false;
    subject.defects.compaction_change_send_fields_fixture = true;
    let report = run(&subject).await;
    for name in NAMES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

async fn rejects(fault: &'static str, name: &str, message: &str) {
    let report = run(&subject(Some(fault))).await;
    let reason = failed(&report, name);
    assert!(reason.contains(message), "{reason}");
    assert_none_failed_but(&report, &[name]);
    for other in NAMES {
        if *other != name {
            assert_eq!(
                report.outcome(other),
                Some(&CaseOutcome::Passed),
                "{}",
                report.render()
            );
        }
    }
}

#[tokio::test]
async fn compaction_add_refusal_fails_only_add_case() {
    rejects("refuse_add", NAMES[0], "must be accepted").await;
}

#[tokio::test]
async fn compaction_remove_acceptance_fails_only_remove_case() {
    rejects("accept_remove", NAMES[1], "not refused").await;
}

#[tokio::test]
async fn compaction_remove_wrong_field_fails_only_remove_case() {
    rejects("wrong_remove_field", NAMES[1], "plan.compaction_item").await;
}

#[tokio::test]
async fn compaction_switch_wrong_field_fails_only_switch_case() {
    rejects("wrong_switch_field", NAMES[2], "plan.compaction_item").await;
}

#[tokio::test]
async fn compaction_changes_without_item_fixture_are_inapplicable() {
    let mut subject = subject(None);
    subject.defects.compaction_change_fixture = false;
    let report = run(&subject).await;
    for name in NAMES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason })
            if reason.contains("compaction_change_plan") && reason.contains("send_fields")),
            "{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn compaction_switch_without_second_fixture_is_inapplicable() {
    let report = run(&subject(Some("no_switch_fixture"))).await;
    assert!(
        matches!(report.outcome(NAMES[2]), Some(CaseOutcome::Inapplicable { reason })
        if reason.contains("compaction_switch_plan") && reason.contains("refresh_plan(ChangedTools)")),
        "{}",
        report.render()
    );
    for name in &NAMES[..2] {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[test]
fn compaction_change_specs_require_plans_refresh_runs_and_reads() {
    for name in NAMES {
        let spec = CASES.iter().find(|spec| spec.name == *name).unwrap();
        for cap in [
            Capability::Plans,
            Capability::SessionChange,
            Capability::Queue,
            Capability::RunOps,
            Capability::TranscriptReads,
        ] {
            assert!(spec.requires.contains(&cap), "{name} needs {cap:?}");
        }
    }
}
