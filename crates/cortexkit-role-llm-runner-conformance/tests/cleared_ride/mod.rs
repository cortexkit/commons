use super::*;

const CASES: &[&str] = &[
    "cleared_ride_confirmed_route",
    "cleared_ride_unconfirmed_falls_through",
    "cleared_ride_free_text_waits",
    "cleared_ride_free_text_falls_through",
    "cleared_ride_route_switch_omits",
    "cleared_ride_persistent_never_upgrades",
    "cleared_ride_plan_notes_never_clear",
];

fn subject(defect: Option<&'static str>) -> FakeSubject {
    let mut subject = note_subject(None);
    subject.defects.cleared_ride = defect;
    subject.turn_system_cleared = true;
    subject.delivery_order_modes = Some(vec![
        "tool_result".into(),
        "turn_system".into(),
        "turn_system_cleared".into(),
        "steer".into(),
        "idle_start".into(),
    ]);
    subject.capabilities.extend([
        Capability::DeliveryOrder,
        Capability::Notices,
        Capability::HistoryDisplay,
        Capability::ReadDisplayFacts,
        Capability::LiveEvents,
    ]);
    subject
}

#[tokio::test]
async fn cleared_ride_faithful_runner_passes() {
    let report = run(&subject(None)).await;
    for name in CASES {
        assert_eq!(
            report.outcome(name),
            Some(&CaseOutcome::Passed),
            "{name}: {}",
            report.render()
        );
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn cleared_ride_default_adapter_skips_all_cases() {
    let mut subject = subject(None);
    subject.turn_system_cleared = false;
    let report = run(&subject).await;
    for name in CASES {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{name}: {}",
            report.render()
        );
    }
}

#[tokio::test]
async fn cleared_ride_requires_explicit_mode_advertisement() {
    for modes in [
        None,
        Some(vec![
            "tool_result".into(),
            "turn_system".into(),
            "steer".into(),
            "idle_start".into(),
        ]),
    ] {
        let mut subject = subject(None);
        subject.delivery_order_modes = modes;
        let report = run(&subject).await;
        for name in CASES {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { reason })
                if reason == "turn_system_cleared delivery is not explicitly declared"),
                "{name}: {}",
                report.render()
            );
        }
        assert_none_failed_but(&report, &[]);
    }
}

async fn rejects(defect: &'static str, cases: &[&str], reason: &str) {
    let report = run(&subject(Some(defect))).await;
    for name in cases {
        let failure = failed(&report, name);
        assert!(failure.contains(reason), "{name}: {failure}");
        println!("FAIL {name}: {failure}");
    }
    assert_none_failed_but(&report, cases);
}

#[tokio::test]
async fn cleared_ride_rejects_wrong_recorded_form() {
    rejects(
        "record",
        &[
            "cleared_ride_confirmed_route",
            "cleared_ride_route_switch_omits",
        ],
        "wrong recorded cleared-ride fact",
    )
    .await;
}

#[tokio::test]
async fn cleared_ride_rejects_unconfirmed_route() {
    rejects(
        "unconfirmed",
        &[
            "cleared_ride_unconfirmed_falls_through",
            "cleared_ride_persistent_never_upgrades",
        ],
        "wrong recorded cleared-ride fact",
    )
    .await;
}

#[tokio::test]
async fn cleared_ride_rejects_free_text_in_system_form() {
    rejects(
        "free_text",
        &[
            "cleared_ride_free_text_waits",
            "cleared_ride_free_text_falls_through",
        ],
        "system authority",
    )
    .await;
}

#[tokio::test]
async fn cleared_ride_rejects_persistent_fallback_after_switch() {
    rejects(
        "switch_persists",
        &["cleared_ride_route_switch_omits"],
        "forbidden system form",
    )
    .await;
}

#[tokio::test]
async fn cleared_ride_rejects_upgrading_persistent_text() {
    rejects(
        "upgrade",
        &[
            "cleared_ride_persistent_never_upgrades",
            "cleared_ride_free_text_falls_through",
        ],
        "system",
    )
    .await;
}

#[tokio::test]
async fn cleared_ride_rejects_clearing_a_plan_note() {
    rejects(
        "plan_note",
        &["cleared_ride_plan_notes_never_clear"],
        "plan note used turn_system_cleared",
    )
    .await;
}
