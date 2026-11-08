use super::*;

fn subject(defect: Option<&'static str>, enabled: bool) -> FakeSubject {
    let mut subject = declaring(
        Capability::DeliveryOrder,
        Defects {
            notice_keep: defect,
            ..Defects::default()
        },
    );
    subject.capabilities.extend([
        Capability::MessageMarks,
        Capability::HostHistory,
        Capability::HistoryDisplay,
        Capability::ReadDisplayFacts,
    ]);
    if enabled {
        subject.capabilities.insert(Capability::NoticeKeep);
    }
    subject
}

#[tokio::test]
async fn notice_keep_faithful_runner_passes() {
    let report = run(&subject(None, true)).await;
    for name in cases_of(&["notice_keep_"]) {
        if name == "notice_keep_undeclared" {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
                "{}",
                report.render()
            );
        } else {
            assert_passed(&report, name);
        }
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn notice_keep_undeclared_runs_only_undeclared_case() {
    let report = run(&subject(None, false)).await;
    assert_passed(&report, "notice_keep_undeclared");
    for name in cases_of(&["notice_keep_"]) {
        if name != "notice_keep_undeclared" {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
                "{}",
                report.render()
            );
        }
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn notice_keep_without_carrier_hook_is_inapplicable_only_for_owner_case() {
    let mut subject = subject(None, true);
    subject.defects.scoped_start_hook_unavailable = true;
    let report = run(&subject).await;
    for name in cases_of(&["notice_keep_"]) {
        if matches!(name, "notice_keep_owner_only" | "notice_keep_undeclared") {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
                "{}",
                report.render()
            );
        } else {
            assert_passed(&report, name);
        }
    }
    assert_none_failed_but(&report, &[]);
}

async fn rejects(defect: &'static str, name: &str, reason: &str) {
    let report = run(&subject(Some(defect), name != "notice_keep_undeclared")).await;
    let failure = failed(&report, name);
    assert!(failure.contains(reason), "{name}: {failure}");
    println!("FAIL {name}: {failure}");
    for other in cases_of(&["notice_keep_"]) {
        if other != name
            && !matches!(
                report.outcome(other),
                Some(CaseOutcome::Inapplicable { .. })
            )
        {
            assert_passed(&report, other);
        }
    }
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn notice_keep_faulty_runner_drops_keep() {
    rejects("drop", "notice_keep_owner_served", "incorrect keep").await;
}

#[tokio::test]
async fn notice_keep_faulty_runner_accepts_without_notice() {
    rejects("notice", "notice_keep_requires_notice", "not refused").await;
}

#[tokio::test]
async fn notice_keep_faulty_runner_accepts_non_owner() {
    rejects("owner", "notice_keep_owner_only", "not refused").await;
}

#[tokio::test]
async fn notice_keep_faulty_runner_writes_before_refusal() {
    rejects(
        "owner_write",
        "notice_keep_owner_only",
        "wrote history, head or baseline",
    )
    .await;
}

#[tokio::test]
async fn notice_keep_faulty_runner_accepts_undeclared_keep() {
    rejects("undeclared", "notice_keep_undeclared", "not refused").await;
}

#[tokio::test]
async fn notice_keep_faulty_runner_stores_false() {
    rejects("false", "notice_keep_false_absent", "incorrect keep").await;
}

#[tokio::test]
async fn notice_keep_faulty_runner_refuses_rides_with_wrong_field() {
    rejects("ride", "notice_keep_tool_result_refused", "keep").await;
}
