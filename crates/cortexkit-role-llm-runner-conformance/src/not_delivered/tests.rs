use super::*;
use crate::{
    drive::Mint,
    fork_fake::{Defects, FakeSubject},
    report::CASES,
};
use cortexkit_role_harness::CrashDriver;

const HANDOVER: &str = "send_not_delivered_handover_failed_final";
const ROUTING: &str = "send_not_delivered_routing_refused_final";
const QUEUED: &str = "send_queued_without_own_run_stays_pending";

async fn outcome(name: &str, fault: Option<&'static str>, status: bool) -> Result<Ending, String> {
    let mut subject = FakeSubject::new(Defects {
        not_delivered: fault,
        ..Defects::default()
    });
    if status {
        subject.capabilities.insert(Capability::SendStatus);
    }
    let root = tempfile::tempdir().unwrap();
    let mut driver = CrashDriver::new(&subject);
    let handle = driver.spawn(&root.path().join("main")).await.unwrap();
    let declared = subject.capabilities();
    let mint = Mint::default();
    let case = Case {
        subject: &subject,
        handle: &handle,
        declared: &declared,
        mint: &mint,
    };
    assert!(CASES.iter().any(|spec| spec.name == name));
    case.run(name)
        .await
        .map_err(|reason| format!("{name}: {reason}"))
}

#[tokio::test]
async fn not_delivered_faithful_runner_passes_both_reasons() {
    for name in [HANDOVER, ROUTING] {
        for status in [false, true] {
            assert!(matches!(
                outcome(name, None, status).await,
                Ok(Ending::Passed)
            ));
        }
    }
}

#[tokio::test]
async fn not_delivered_redelivery_fails_each_named_check() {
    for name in [HANDOVER, ROUTING] {
        let reason = outcome(name, Some("redeliver"), true).await.err().unwrap();
        assert!(reason.contains(name), "{reason}");
        assert!(reason.contains("final answer"), "{reason}");
        eprintln!("{reason}");
    }
}

#[tokio::test]
async fn not_delivered_changed_reason_and_receipt_fail_by_name() {
    for name in [HANDOVER, ROUTING] {
        for (fault, expected) in [
            ("reason_changes", "final answer"),
            ("receipt", "delivered to be absent"),
        ] {
            let reason = outcome(name, Some(fault), true).await.err().unwrap();
            assert!(
                reason.contains(name) && reason.contains(expected),
                "{reason}"
            );
        }
    }
}

#[tokio::test]
async fn not_delivered_allocated_run_requires_error_with_refusal() {
    for fault in ["run_not_error", "run_no_refusal"] {
        let reason = outcome(HANDOVER, Some(fault), true).await.err().unwrap();
        assert!(
            reason.contains(HANDOVER) && reason.contains("error carrying its refusal"),
            "{reason}"
        );
    }
}

#[tokio::test]
async fn not_delivered_unavailable_fixtures_are_inapplicable() {
    for name in [HANDOVER, ROUTING, QUEUED] {
        assert!(matches!(
            outcome(name, Some("no_fixture"), true).await,
            Ok(Ending::Inapplicable(_))
        ));
    }
}

#[tokio::test]
async fn not_delivered_queued_without_run_stays_pending_and_bad_runner_fails_by_name() {
    assert!(matches!(
        outcome(QUEUED, Some("pending_run_id"), true).await,
        Ok(Ending::Passed)
    ));
    assert!(matches!(
        outcome(QUEUED, None, true).await,
        Ok(Ending::Passed)
    ));
    let reason = outcome(QUEUED, Some("queue_failed"), true)
        .await
        .err()
        .unwrap();
    assert!(
        reason.contains(QUEUED) && reason.contains("must stay pending"),
        "{reason}"
    );
    eprintln!("{reason}");
}
