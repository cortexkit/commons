use super::*;

const NAME: &str = "subscribe_resume_refusals";

fn subject(defects: Defects, live: bool) -> FakeSubject {
    let mut subject = FakeSubject::new(defects);
    subject.kill_points.clear();
    if live {
        subject.capabilities.insert(Capability::LiveEvents);
    }
    subject
}

#[tokio::test]
async fn subscribe_resume_refusals_faithful_runner_passes() {
    for live in [false, true] {
        let report = run(&subject(Defects::default(), live)).await;
        assert_passed(&report, NAME);
        assert_none_failed_but(&report, &[]);
    }
}

#[tokio::test]
async fn subscribe_resume_refusals_rejects_accepted_malformed_from() {
    for live in [false, true] {
        let report = run(&subject(
            Defects {
                subscribe_malformed_from_accepted: true,
                ..Defects::default()
            },
            live,
        ))
        .await;
        let reason = failed(&report, NAME);
        assert!(reason.contains("malformed from 3"), "{reason}");
        assert_none_failed_but(&report, &[NAME]);
    }
}

#[tokio::test]
async fn subscribe_resume_refusals_rejects_invalid_params_for_foreign_cursor() {
    for live in [false, true] {
        let report = run(&subject(
            Defects {
                subscribe_foreign_cursor_invalid_params: true,
                ..Defects::default()
            },
            live,
        ))
        .await;
        let reason = failed(&report, NAME);
        assert!(
            reason.contains("foreign-session resume cursor was not refused cursor_expired"),
            "{reason}"
        );
        assert_none_failed_but(&report, &[NAME]);
    }
}
