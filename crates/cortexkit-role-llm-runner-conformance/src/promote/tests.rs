use super::*;
use crate::{
    fork_fake::{Defects, FakeSubject},
    report::CASES,
};

async fn outcome(
    name: &str,
    fault: Option<&'static str>,
    group: bool,
    steer: bool,
) -> Result<Ending, String> {
    let mut subject = FakeSubject::new(Defects {
        promote: fault,
        ..Defects::default()
    });
    outcome_with_subject(name, &mut subject, group, steer).await
}

async fn outcome_with_subject(
    name: &str,
    subject: &mut FakeSubject,
    group: bool,
    steer: bool,
) -> Result<Ending, String> {
    if group {
        subject.capabilities.insert(Capability::SendPromote);
    } else {
        subject.capabilities.remove(&Capability::SendPromote);
    }
    subject.capabilities.insert(Capability::DeliveryOrder);
    subject.capabilities.insert(Capability::SendStatus);
    if !steer {
        subject.capabilities.remove(&Capability::Steer);
    }
    let root = tempfile::tempdir().unwrap();
    let mut driver = CrashDriver::new(subject);
    let handle = driver.spawn(&root.path().join("main")).await.unwrap();
    let declared = subject.capabilities();
    let mint = Mint::default();
    assert!(CASES.iter().any(|spec| spec.name == name));
    let result = if name == REPLAY {
        restart(subject, &mut driver, root.path(), &declared, &mint)
            .await
            .map(|()| Ending::Passed)
    } else {
        Case {
            subject,
            handle: &handle,
            declared: &declared,
            mint: &mint,
        }
        .run(name)
        .await
    };
    result.map_err(|reason| format!("{name}: {reason}"))
}

#[tokio::test]
async fn send_promote_turn_ended_first_waits_for_early_turn_receipt_to_finish() {
    let mut subject = FakeSubject::new(Defects::default());
    subject.early_idle_turn_receipt = true;
    let result =
        outcome_with_subject("send_promote_turn_ended_first", &mut subject, true, true).await;
    assert!(
        matches!(result, Ok(Ending::Passed)),
        "{}",
        result.err().unwrap_or_default()
    );
}

#[tokio::test]
async fn send_promote_faithful_runner_reaches_every_answer() {
    for name in [
        "send_promote_active_next_step",
        "send_promote_turn_ended_first",
        "send_promote_run_ends_before_step",
        "send_promote_parked",
        "send_promote_sender_namespace",
        "send_promote_steer_admission",
        REPLAY,
        "send_promote_requires_steer",
    ] {
        let result = outcome(name, None, true, true).await;
        assert!(
            matches!(result, Ok(Ending::Passed)),
            "{name}: {}",
            result.err().unwrap_or_default()
        );
    }
}

#[tokio::test]
async fn send_promote_incorrect_runners_fail_each_named_check() {
    for (name, fault, expected) in [
        (
            "send_promote_active_next_step",
            "no_delivery",
            "step receipt",
        ),
        (
            "send_promote_turn_ended_first",
            "ended_first",
            "expected Delivered",
        ),
        (
            "send_promote_run_ends_before_step",
            "drop_at_end",
            "turn receipt",
        ),
        ("send_promote_parked", "parked", "expected NotPromoted"),
        (
            "send_promote_sender_namespace",
            "foreign",
            "expected NotQueued",
        ),
        (
            "send_promote_sender_namespace",
            "unknown",
            "expected NotQueued",
        ),
        ("send_promote_steer_admission", "admission", "not refused"),
        (REPLAY, "replay", "expected NotPromoted"),
        (REPLAY, "reuse", "not refused"),
        (REPLAY, "volatile", "expected NotPromoted"),
    ] {
        let reason = outcome(name, Some(fault), true, true)
            .await
            .err()
            .expect("incorrect runner passed");
        assert!(
            reason.contains(name) && reason.contains(expected),
            "{reason}"
        );
        eprintln!("{fault}: {reason}");
    }
}

#[tokio::test]
async fn send_promote_unsupported_without_group_or_steer() {
    for (group, steer) in [(false, true), (false, false), (true, false)] {
        assert!(matches!(
            outcome("send_promote_unsupported", None, group, steer).await,
            Ok(Ending::Passed)
        ));
        let reason = outcome(
            "send_promote_unsupported",
            Some("unsupported"),
            group,
            steer,
        )
        .await
        .err()
        .expect("incorrect runner passed");
        assert!(reason.contains("send_promote_unsupported"), "{reason}");
    }
    let reason = outcome("send_promote_requires_steer", None, true, false)
        .await
        .err()
        .expect("invalid declaration passed");
    assert!(
        reason.contains("send_promote_requires_steer")
            && reason.contains("send_promote_without_steer"),
        "{reason}"
    );
}
