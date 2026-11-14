use super::*;

const CASES: &[&str] = &["account_head_turn_runner", "account_head_turn_non_runner"];

#[tokio::test]
async fn account_head_turn_cases_pass_and_gate_on_capability() {
    for declared in [true, false] {
        let subject = if declared {
            FakeSubject::new(Defects::default())
        } else {
            FakeSubject::without_extensions(Defects::default())
        };
        let report = run_suite(&subject).await;
        assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
        for name in CASES {
            match report.outcome(name) {
                Some(CheckOutcome::Passed) if declared => {}
                Some(CheckOutcome::NotApplicable { reason }) if !declared => {
                    assert!(!reason.is_empty())
                }
                _ => panic!("{name}: {}", report.render()),
            }
        }
    }
}

#[tokio::test]
async fn account_head_turn_faulty_routers_fail_by_name() {
    for (fault, expected) in [
        ("account_head_turn_runner", CASES[0]),
        ("account_head_turn_not_counted", CASES[0]),
        ("account_head_turn_non_runner", CASES[1]),
    ] {
        let subject = FakeSubject::new(Defects {
            extension: Some(fault),
            ..Defects::default()
        });
        let report = run_suite(&subject).await;
        assert_eq!(report.failing(), [expected], "{}", report.render());
        println!(
            "rejected wrong router: {expected}: {:?}",
            report.outcome(expected)
        );
    }
}
