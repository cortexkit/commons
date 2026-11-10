use super::select_engine::fake::{Defects, FakeSubject};
use super::*;
use crate::{CheckOutcome, CHECKS};

const CASES: &[&str] = &[
    "account_observe_credit_without_reset_accepted",
    "continue_credit_window_refused",
];

#[tokio::test]
async fn credit_observation_and_quota_only_continuation_pass() {
    for subject in [
        FakeSubject::new(Defects::default()),
        FakeSubject::without_extensions(Defects::default()),
    ] {
        for name in CASES {
            assert_eq!(
                Suite::new(&subject).run(name).await,
                Ok(Ending::Passed),
                "{name}"
            );
        }
    }
}

#[tokio::test]
async fn credit_faulty_routers_are_rejected_by_check_name() {
    for name in CASES {
        let subject = FakeSubject::new(Defects {
            extension: Some(name),
            ..Defects::default()
        });
        let spec = CHECKS.iter().find(|spec| spec.name == *name).unwrap();
        let report = crate::runner::run_check(&Suite::new(&subject), spec).await;
        assert_eq!(report.check, *name);
        assert!(
            matches!(report.outcome, CheckOutcome::Failed { .. }),
            "faulty router must fail {name}: {report:?}"
        );
    }
}
