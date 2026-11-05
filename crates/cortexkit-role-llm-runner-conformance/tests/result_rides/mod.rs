use super::*;

fn subject(defect: Option<&'static str>, enabled: bool) -> FakeSubject {
    let mut subject = declaring(
        Capability::DeliveryOrder,
        Defects {
            result_rides: defect,
            ..Defects::default()
        },
    );
    subject.capabilities.extend([
        Capability::HostHistory,
        Capability::HistoryDisplay,
        Capability::ReadDisplayFacts,
        Capability::LiveEvents,
    ]);
    if enabled {
        subject.capabilities.insert(Capability::ResultRides);
    }
    subject
}

#[tokio::test]
async fn result_rides_faithful_runner_passes() {
    let report = run(&subject(None, true)).await;
    for name in cases_of(&["result_rides_"]) {
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
async fn result_rides_undeclared_is_inapplicable() {
    let report = run(&subject(None, false)).await;
    for name in cases_of(&["result_rides_"]) {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{name}: {}",
            report.render()
        );
    }
}

async fn rejects(defect: &'static str, name: &str, reason: &str) {
    let report = run(&subject(Some(defect), true)).await;
    let failure = failed(&report, name);
    assert!(failure.contains(reason), "{name}: {failure}");
    for other in cases_of(&["result_rides_"]) {
        if other != name {
            assert_eq!(
                report.outcome(other),
                Some(&CaseOutcome::Passed),
                "{}",
                report.render()
            );
        }
    }
    println!("FAIL {name}: {failure}");
}

#[tokio::test]
async fn result_rides_rejects_transcript_entry() {
    rejects(
        "entry",
        "result_rides_no_transcript_entry",
        "created a transcript entry",
    )
    .await;
}

#[tokio::test]
async fn result_rides_rejects_stored_output_mutation() {
    rejects(
        "stored",
        "result_rides_stored_output_unchanged",
        "host stored tool output changed",
    )
    .await;
}

#[tokio::test]
async fn result_rides_rejects_wrong_fact() {
    rejects("fact", "result_rides_display_fact", "wrong rides fact").await;
}

#[tokio::test]
async fn result_rides_rejects_missing_live_event() {
    rejects("event", "result_rides_live_event", "ride_delivered event").await;
}

#[tokio::test]
async fn result_rides_rejects_wrong_append_bytes() {
    rejects("bytes", "result_rides_append_bytes", "exact append bytes").await;
}

#[tokio::test]
async fn result_rides_rejects_system_authority() {
    rejects(
        "system",
        "result_rides_tool_result_only_never_system",
        "reached system authority",
    )
    .await;
}
