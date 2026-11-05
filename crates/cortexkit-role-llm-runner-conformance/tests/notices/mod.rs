use super::*;

fn subject(defect: Option<&'static str>, enabled: bool) -> FakeSubject {
    let mut subject = declaring(
        Capability::DeliveryOrder,
        Defects {
            notices: defect,
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
        subject.capabilities.insert(Capability::Notices);
    }
    subject
}

#[tokio::test]
async fn notices_faithful_runner_passes() {
    let report = run(&subject(None, true)).await;
    for name in cases_of(&["notices_"]) {
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
async fn notices_undeclared_is_inapplicable() {
    let report = run(&subject(None, false)).await;
    for name in cases_of(&["notices_"]) {
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
    for other in cases_of(&["notices_"]) {
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
async fn notices_rejects_transcript_entry() {
    rejects(
        "entry",
        "notices_no_transcript_entry",
        "created a transcript entry",
    )
    .await;
}

#[tokio::test]
async fn notices_rejects_stored_output_mutation() {
    rejects(
        "stored",
        "notices_stored_output_unchanged",
        "host stored tool output changed",
    )
    .await;
}

#[tokio::test]
async fn notices_rejects_missing_fact_call_key() {
    rejects(
        "missing_call_key",
        "notices_display_fact",
        "missing field `call_key`",
    )
    .await;
}

#[tokio::test]
async fn notices_rejects_wrong_fact() {
    rejects("fact", "notices_display_fact", "wrong notices fact").await;
}

#[tokio::test]
async fn notices_rejects_missing_live_event() {
    rejects("event", "notices_live_event", "notice_delivered event").await;
}

#[tokio::test]
async fn notices_rejects_wrong_append_bytes() {
    rejects("bytes", "notices_append_bytes", "exact append bytes").await;
}

#[tokio::test]
async fn notices_rejects_system_authority() {
    rejects(
        "system",
        "notices_tool_result_only_never_system",
        "reached system authority",
    )
    .await;
}
