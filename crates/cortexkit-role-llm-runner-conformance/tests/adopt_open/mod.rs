use super::*;

const CASE_NAMES: &[&str] = &[
    "adopt_open_opens_unknown_session",
    "adopt_open_repeat_replays",
    "adopt_open_existing_session_unchanged",
    "adopt_open_non_owner_refused",
];

fn subject(defect: Option<&'static str>) -> FakeSubject {
    let mut subject = FakeSubject::new(Defects {
        adopt_open: defect,
        ..Defects::default()
    });
    subject.capabilities.extend([
        Capability::Adopt,
        Capability::AdoptOpen,
        Capability::Plans,
        Capability::DeliveryOrder,
    ]);
    subject.head_sender_fixtures = true;
    subject
}

#[tokio::test]
async fn adopt_open_faithful_runner_passes() {
    let report = run(&subject(None)).await;
    for name in CASE_NAMES {
        assert_passed(&report, name);
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn adopt_open_faulty_runners_fail_by_name() {
    for (fault, name) in [
        ("write_message", "adopt_open_opens_unknown_session"),
        ("ignore_plan", "adopt_open_opens_unknown_session"),
        ("unknown_until_message", "adopt_open_opens_unknown_session"),
        ("omit_sender_grant", "adopt_open_opens_unknown_session"),
        ("reuse_plan", "adopt_open_repeat_replays"),
        ("non_owner", "adopt_open_non_owner_refused"),
        ("reapply", "adopt_open_existing_session_unchanged"),
    ] {
        let report = run(&subject(Some(fault))).await;
        let reason = failed(&report, name);
        eprintln!("FAIL {name} ({fault}): {reason}");
    }
}

#[tokio::test]
async fn adopt_open_without_group_is_inapplicable_and_sends_no_adopts() {
    let subject = FakeSubject::new(Defects::default());
    let report = run(&subject).await;
    for name in CASE_NAMES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{name}: {}",
            report.render()
        );
    }
    assert_eq!(subject.adopt_calls(), 0);
}

#[tokio::test]
async fn adopt_open_without_carrier_hook_only_carrier_case_is_inapplicable() {
    let mut subject = subject(None);
    subject.defects.scoped_start_hook_unavailable = true;
    let report = run(&subject).await;
    for name in CASE_NAMES {
        if *name == "adopt_open_non_owner_refused" {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
                "{}",
                report.render()
            );
        } else {
            assert_passed(&report, name);
        }
    }
}

#[tokio::test]
async fn adopt_open_without_head_sender_only_grant_rule_is_inapplicable() {
    let mut subject = subject(None);
    subject.head_sender_group = false;
    let report = run(&subject).await;
    assert!(
        matches!(report.outcome(CASE_NAMES[0]), Some(CaseOutcome::PassedWithInapplicableRule { reason }) if reason.contains("head_sender")),
        "{}",
        report.render()
    );
    for name in &CASE_NAMES[1..] {
        assert_passed(&report, name);
    }
}

#[tokio::test]
async fn adopt_open_without_delivery_order_only_grant_rule_is_inapplicable() {
    let mut subject = subject(None);
    subject.capabilities.remove(&Capability::DeliveryOrder);
    subject.head_sender_fixtures = false;
    let report = run(&subject).await;
    assert!(
        matches!(report.outcome(CASE_NAMES[0]), Some(CaseOutcome::PassedWithInapplicableRule { reason }) if reason.contains("delivery_order")),
        "{}",
        report.render()
    );
    for name in &CASE_NAMES[1..] {
        assert_passed(&report, name);
    }
}
