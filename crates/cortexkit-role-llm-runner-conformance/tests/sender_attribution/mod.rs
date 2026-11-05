use super::*;

fn subject(defect: Option<&'static str>, enabled: bool) -> FakeSubject {
    let mut subject = declaring(
        Capability::DeliveryOrder,
        Defects {
            attribution: defect,
            ..Defects::default()
        },
    );
    subject.capabilities.extend([
        Capability::HostHistory,
        Capability::HistoryDisplay,
        Capability::ReadDisplayFacts,
        Capability::Attachments,
        Capability::MediaOnlySend,
    ]);
    if enabled {
        subject.capabilities.insert(Capability::SenderAttribution);
    }
    subject
}

#[tokio::test]
async fn sender_attribution_faithful_runner_passes() {
    let report = run(&subject(None, true)).await;
    for name in cases_of(&["sender_attribution_"]) {
        if name != "sender_attribution_undeclared_refused" {
            assert_eq!(
                report.outcome(name),
                Some(&CaseOutcome::Passed),
                "{}",
                report.render()
            );
        }
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn sender_attribution_each_fault_rejected_by_its_case_name() {
    for (defect, name) in [
        ("missing_header", "sender_attribution_stored_header"),
        ("wrong_fact", "sender_attribution_fact"),
        ("photo_no_header", "sender_attribution_photo_only"),
        ("template_header", "sender_attribution_template"),
        ("other_sender_header", "sender_attribution_other_sender"),
        ("stale_fact", "sender_attribution_tampered_prefix"),
        ("display_leak", "sender_attribution_display"),
        (
            "undeclared_accepted",
            "sender_attribution_undeclared_refused",
        ),
    ] {
        let report = run(&subject(Some(defect), defect != "undeclared_accepted")).await;
        assert!(
            !failed(&report, name).contains("refused invalid_params"),
            "{name}: the deliberately faulty rule was not reached: {}",
            report.render()
        );
        assert!(
            !failed(&report, name).is_empty(),
            "{name}: {}",
            report.render()
        );
    }
}

#[tokio::test]
async fn sender_attribution_undeclared_runner_refuses_plan() {
    let report = run(&subject(None, false)).await;
    assert_eq!(
        report.outcome("sender_attribution_undeclared_refused"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    for name in cases_of(&["sender_attribution_"]) {
        if name != "sender_attribution_undeclared_refused" {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
                "{name}"
            );
        }
    }
}
