use super::*;

const NAME: &str = "message_senders_rows_name_their_sender";

fn subject(mut defects: Defects) -> FakeSubject {
    defects.declare_message_senders = true;
    let mut subject = declaring(Capability::DeliveryOrder, defects);
    subject.capabilities.extend([
        Capability::HostHistory,
        Capability::HistoryDisplay,
        Capability::LiveEvents,
        Capability::ClientMessageId,
        Capability::ReadDisplayFacts,
    ]);
    subject.head_sender_fixtures = true;
    subject
}

fn send_ids_subject(defects: Defects) -> FakeSubject {
    let mut subject = subject(defects);
    subject.capabilities.insert(Capability::SendIds);
    subject
}

#[tokio::test]
async fn message_senders_missing_read_sender_fails_by_name() {
    let report = run(&subject(Defects {
        sender_read_missing: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, NAME);
    assert!(reason.contains("read entry sender is None"), "{reason}");
    assert_none_failed_but(&report, &[NAME]);
    println!("FAIL {NAME}: {reason}");
}

#[tokio::test]
async fn message_senders_without_read_display_facts_still_checks_rows() {
    let mut subject = subject(Defects {
        sender_read_missing: true,
        ..Defects::default()
    });
    subject.capabilities.remove(&Capability::ReadDisplayFacts);
    let report = run(&subject).await;
    assert_passed(&report, NAME);
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn message_senders_correct_runner_passes_start_and_join() {
    let report = run(&subject(Defects::default())).await;
    assert_eq!(
        report.outcome(NAME),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn message_senders_omitted_sender_fails_by_name() {
    let report = run(&subject(Defects {
        message_senders_omit: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        failed(&report, NAME).contains("sender is None"),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[NAME]);
}

#[tokio::test]
async fn message_senders_owner_instead_of_sender_b_fails_by_name() {
    let report = run(&subject(Defects {
        message_senders_owner: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, NAME);
    assert!(
        reason.contains("sender is Some") && reason.contains("fake.sender-b"),
        "{reason}"
    );
    assert_none_failed_but(&report, &[NAME]);
}

#[tokio::test]
async fn message_senders_late_binding_passes_buffered_checks_but_fails_live_timing_by_name() {
    let report = run(&subject(Defects {
        message_senders_late: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        failed(&report, NAME).contains("message_senders_timing_at_message_started: sender is None"),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[NAME]);
}

#[tokio::test]
async fn message_senders_join_binding_is_checked_after_the_start_passes() {
    for late in [false, true] {
        let report = run(&subject(Defects {
            message_senders_join_only: true,
            message_senders_omit: !late,
            message_senders_late: late,
            ..Defects::default()
        }))
        .await;
        let reason = failed(&report, NAME);
        assert!(reason.contains("sender is None"), "{reason}");
        assert_eq!(
            reason.contains("message_senders_timing_at_message_started"),
            late,
            "{reason}"
        );
        assert_none_failed_but(&report, &[NAME]);
    }
}

#[tokio::test]
async fn message_senders_model_reply_with_sender_fails_by_name() {
    let report = run(&subject(Defects {
        message_senders_model: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        failed(&report, NAME).contains("model reply"),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[NAME]);
}

#[tokio::test]
async fn message_senders_missing_client_message_id_fails_by_name() {
    let report = run(&subject(Defects {
        message_senders_no_client_id: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        failed(&report, NAME).contains("client_message_id is None"),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[NAME]);
}

#[tokio::test]
async fn message_senders_without_live_observer_still_checks_buffered_rows() {
    let report = run(&subject(Defects {
        message_senders_no_observer: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        matches!(report.outcome(NAME), Some(CaseOutcome::PassedWithInapplicableRule { reason }) if reason.contains("message_senders_timing_at_message_started")),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
    let report = run(&subject(Defects {
        message_senders_no_observer: true,
        message_senders_omit: true,
        ..Defects::default()
    }))
    .await;
    assert!(
        failed(&report, NAME).contains("sender is None"),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[NAME]);
}

#[tokio::test]
async fn message_senders_without_a_steerable_hold_checks_start_only() {
    let mut subject = subject(Defects::default());
    subject.capabilities.remove(&Capability::Steer);
    let report = run(&subject).await;
    assert!(
        matches!(report.outcome(NAME), Some(CaseOutcome::PassedWithInapplicableRule { reason }) if reason.contains("message_senders_join") && !reason.contains("message_senders_timing")),
        "{}",
        report.render()
    );
}

#[tokio::test]
async fn message_senders_undeclared_group_and_missing_identities_are_inapplicable_by_name() {
    for no_group in [true, false] {
        let mut subject = subject(Defects::default());
        if no_group {
            subject.defects.declare_message_senders = false;
        } else {
            subject.head_sender_fixtures = false;
        }
        let report = run(&subject).await;
        let missing = if no_group {
            "message_senders"
        } else {
            "head_sender_identities"
        };
        assert!(
            matches!(report.outcome(NAME), Some(CaseOutcome::Inapplicable { reason }) if reason.contains(missing)),
            "{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn message_senders_missing_dependencies_are_inapplicable_by_name() {
    for dependency in [Capability::HistoryDisplay, Capability::LiveEvents] {
        let mut subject = subject(Defects::default());
        subject.capabilities.remove(&dependency);
        let report = run(&subject).await;
        assert!(
            matches!(report.outcome(NAME), Some(CaseOutcome::Inapplicable { reason }) if reason.contains(&dependency.name())),
            "{}",
            report.render()
        );
    }
}

#[tokio::test]
async fn read_display_facts_send_id_faithful_runner_covers_keyed_and_unkeyed() {
    let report = run(&send_ids_subject(Defects::default())).await;
    assert_eq!(
        report.outcome("read_display_facts_send_id"),
        Some(&CaseOutcome::Passed),
        "{}",
        report.render()
    );
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn read_display_facts_send_id_wrong_read_id_fails_by_name() {
    let report = run(&send_ids_subject(Defects {
        send_id_read_wrong: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "read_display_facts_send_id");
    assert!(reason.contains("read entry send_id"), "{reason}");
    assert_none_failed_but(&report, &["read_display_facts_send_id"]);
    println!("FAIL read_display_facts_send_id: {reason}");
}

#[tokio::test]
async fn read_display_facts_send_id_wrong_history_id_fails_by_name() {
    let report = run(&send_ids_subject(Defects {
        send_id_history_wrong: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "read_display_facts_send_id");
    assert!(reason.contains("history entry send_id"), "{reason}");
    assert_none_failed_but(&report, &["read_display_facts_send_id"]);
    println!("FAIL read_display_facts_send_id: {reason}");
}

#[tokio::test]
async fn read_display_facts_send_id_unkeyed_internal_id_fails_by_name() {
    let report = run(&send_ids_subject(Defects {
        send_id_unkeyed_internal: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "read_display_facts_send_id");
    assert!(
        reason.contains("read entry send_id") && reason.contains("internal-unkeyed"),
        "{reason}"
    );
    assert_none_failed_but(&report, &["read_display_facts_send_id"]);
    println!("FAIL read_display_facts_send_id: {reason}");
}

#[tokio::test]
async fn read_display_facts_send_id_optional_unkeyed_policies_are_not_failures() {
    for policy in ["refused", "no_id"] {
        let report = run(&send_ids_subject(Defects {
            send_id_unkeyed: Some(policy),
            ..Defects::default()
        }))
        .await;
        assert!(
            matches!(report.outcome("read_display_facts_send_id"), Some(CaseOutcome::PassedWithInapplicableRule { reason }) if reason.contains("read_display_facts_send_id_unkeyed")),
            "{}",
            report.render()
        );
        assert_none_failed_but(&report, &[]);
    }
}

#[tokio::test]
async fn read_display_facts_send_id_missing_from_declaring_runner_fails_by_name() {
    let report = run(&send_ids_subject(Defects {
        send_id_missing: true,
        ..Defects::default()
    }))
    .await;
    let reason = failed(&report, "read_display_facts_send_id");
    assert!(reason.contains("read entry send_id is None"), "{reason}");
    assert_none_failed_but(&report, &["read_display_facts_send_id"]);
}

#[tokio::test]
async fn read_display_facts_send_id_undeclared_accepts_absence_or_correct_values() {
    for missing in [true, false] {
        let report = run(&subject(Defects {
            send_id_missing: missing,
            ..Defects::default()
        }))
        .await;
        assert_eq!(
            report.outcome("read_display_facts_send_id_when_present"),
            Some(&CaseOutcome::Passed),
            "{}",
            report.render()
        );
        assert!(
            matches!(report.outcome("read_display_facts_send_id"), Some(CaseOutcome::Inapplicable { reason }) if reason.contains("send_ids")),
            "{}",
            report.render()
        );
        assert_none_failed_but(&report, &[]);
    }
}

#[tokio::test]
async fn read_display_facts_send_id_undeclared_rejects_wrong_present_values() {
    for history in [false, true] {
        let report = run(&subject(Defects {
            send_id_read_wrong: !history,
            send_id_history_wrong: history,
            ..Defects::default()
        }))
        .await;
        let name = "read_display_facts_send_id_when_present";
        let reason = failed(&report, name);
        let op = if history { "history" } else { "read" };
        assert!(reason.contains(&format!("{op} entry send_id")), "{reason}");
        assert_none_failed_but(&report, &[name]);
        println!("FAIL {name}: {reason}");
    }
}
