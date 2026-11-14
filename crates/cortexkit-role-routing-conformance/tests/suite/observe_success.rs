use super::*;

const CASES: &[&str] = &[
    "account_observe_success_clears",
    "account_observe_success_newer_block",
    "account_observe_success_other_credential",
    "account_observe_success_other_account",
    "account_observe_success_unbound",
    "account_observe_success_unknown",
    "account_observe_success_status_refusals",
    "account_observe_success_companions",
    "account_observe_success_kind",
    "account_observe_success_retry",
    "account_observe_success_duplicate",
    "account_observe_success_undeclared",
];

#[tokio::test]
async fn account_observe_success_cases_pass_and_gate_on_capability() {
    for declared in [true, false] {
        let subject = if declared {
            FakeSubject::new(Defects::default())
        } else {
            FakeSubject::without_extensions(Defects::default())
        };
        let report = run_suite(&subject).await;
        assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
        for name in CASES {
            let applicable = declared != (*name == "account_observe_success_undeclared");
            match report.outcome(name) {
                Some(CheckOutcome::Passed) if applicable => {}
                Some(CheckOutcome::NotApplicable { reason }) if !applicable => {
                    assert!(!reason.is_empty())
                }
                other => panic!("{name}: {other:?}\n{}", report.render()),
            }
        }
    }
}

async fn wrong_router(name: &'static str) {
    let defects = Defects {
        extension: Some(name),
        ..Defects::default()
    };
    let subject = if name == "account_observe_success_undeclared" {
        FakeSubject::without_extensions(defects)
    } else {
        FakeSubject::new(defects)
    };
    let report = run_suite(&subject).await;
    assert!(
        matches!(report.outcome(name), Some(CheckOutcome::Failed { .. })),
        "wrong router escaped {name}\n{}",
        report.render()
    );
    println!("rejected wrong router: {name}: {:?}", report.outcome(name));
    assert!(
        report.failing().iter().all(|failed| CASES.contains(failed)),
        "unrelated failure: {}",
        report.render()
    );
}

macro_rules! rejects_wrong_routers {
    ($($name:ident),+ $(,)?) => {$(
        #[tokio::test]
        async fn $name() {
            wrong_router(stringify!($name)).await;
        }
    )+};
}

rejects_wrong_routers! {
    account_observe_success_clears,
    account_observe_success_newer_block,
    account_observe_success_other_credential,
    account_observe_success_other_account,
    account_observe_success_unbound,
    account_observe_success_unknown,
    account_observe_success_status_refusals,
    account_observe_success_companions,
    account_observe_success_kind,
    account_observe_success_retry,
    account_observe_success_duplicate,
    account_observe_success_undeclared,
}

#[tokio::test]
async fn input_characters_wrong_router_fails_by_check_name() {
    let report = run_suite(&FakeSubject::with_media(Defects {
        extension: Some("select_media_input_characters"),
        ..Defects::default()
    }))
    .await;
    assert_eq!(
        report.failing(),
        ["select_media_input_characters"],
        "{}",
        report.render()
    );
    assert!(matches!(
        report.outcome("select_media_input_characters"),
        Some(CheckOutcome::Failed { .. })
    ));
    println!(
        "rejected wrong router: select_media_input_characters: {:?}",
        report.outcome("select_media_input_characters")
    );
}

#[tokio::test]
async fn input_characters_correct_router_passes() {
    let report = run_suite(&FakeSubject::with_media(Defects::default())).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    assert_eq!(
        report.outcome("select_media_input_characters"),
        Some(&CheckOutcome::Passed)
    );
}
