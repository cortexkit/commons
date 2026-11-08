use super::*;

fn subject(defect: Option<&'static str>, enabled: bool) -> FakeSubject {
    let mut subject = declaring(
        Capability::RunToolCalls,
        Defects {
            tag_read: defect,
            declare_message_senders: true,
            ..Defects::default()
        },
    );
    subject.capabilities.extend([
        Capability::MessageMarks,
        Capability::ClientMessageId,
        Capability::HistoryDisplay,
        Capability::ReadDisplayFacts,
        Capability::LiveEvents,
    ]);
    if enabled {
        subject.capabilities.insert(Capability::TagRead);
    }
    subject
}

#[tokio::test]
async fn tag_read_faithful_runner_passes() {
    let report = run(&subject(None, true)).await;
    for name in cases_of(&["tag_read_"]) {
        if name == "tag_read_undeclared" {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
                "{}",
                report.render()
            );
        } else {
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
async fn tag_read_undeclared_runs_only_undeclared_case() {
    let report = run(&subject(None, false)).await;
    assert_passed(&report, "tag_read_undeclared");
    for name in cases_of(&["tag_read_"]) {
        if name != "tag_read_undeclared" {
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
async fn tag_read_missing_cut_or_undo_hook_is_honestly_inapplicable() {
    let mut subject = subject(None, true);
    subject.cannot_compact_in_host = true;
    subject.cannot_revert = true;
    let report = run(&subject).await;
    for name in ["tag_read_before_cut", "tag_read_before_undo"] {
        assert!(
            matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
            "{}",
            report.render()
        );
    }
    assert_eq!(
        report.outcome("tag_read_person"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

async fn rejects(defect: &'static str, name: &str, reason: &str) {
    let report = run(&subject(Some(defect), name != "tag_read_undeclared")).await;
    let failure = failed(&report, name);
    assert!(failure.contains(reason), "{name}: {failure}");
    println!("FAIL {name}: {failure}");
    for other in cases_of(&["tag_read_"]) {
        if other != name && other != "tag_read_undeclared" && name != "tag_read_undeclared" {
            assert_eq!(
                report.outcome(other),
                Some(&CaseOutcome::Passed),
                "{}",
                report.render()
            );
        }
    }
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn tag_read_faulty_runner_returns_wrong_part() {
    rejects(
        "part",
        "tag_read_person",
        "wrong entry, part or display facts",
    )
    .await;
}

#[tokio::test]
async fn tag_read_faulty_runner_substitutes_send_id_for_app_id() {
    rejects(
        "client_id",
        "tag_read_person",
        "wrong entry, part or display facts",
    )
    .await;
}
#[tokio::test]
async fn tag_read_faulty_runner_forgets_cut_tags() {
    rejects("cut", "tag_read_before_cut", "unknown_tag").await;
}
#[tokio::test]
async fn tag_read_faulty_runner_answers_removed_call() {
    rejects("removed_call", "tag_read_before_undo", "not refused").await;
}
#[tokio::test]
async fn tag_read_faulty_runner_misnames_unknown_call() {
    rejects("unknown_call", "tag_read_unknown_call", "not unknown_call").await;
}
#[tokio::test]
async fn tag_read_faulty_runner_answers_unknown_tag() {
    rejects("unknown_tag", "tag_read_unknown_tag", "not refused").await;
}
#[tokio::test]
async fn tag_read_faulty_runner_admits_non_owner() {
    rejects("owner", "tag_read_non_owner", "not scope_owner_mismatch").await;
}
#[tokio::test]
async fn tag_read_faulty_runner_accepts_undeclared_selector() {
    rejects("undeclared", "tag_read_undeclared", "not invalid_params").await;
}
