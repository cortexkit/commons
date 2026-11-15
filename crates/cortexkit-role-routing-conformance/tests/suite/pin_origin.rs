use super::*;

const CASES: &[&str] = &[
    "select_pin_origin_enrolled",
    "select_pin_origin_unenrolled",
    "select_pin_origin_without_pin",
    "select_pin_origin_unknown",
];

#[tokio::test]
async fn select_pin_origin_cases_pass_and_gate_on_capability() {
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
async fn select_pin_origin_faulty_routers_fail_by_name() {
    for (fault, expected) in [
        (CASES[0], CASES[0]),
        (CASES[1], CASES[1]),
        ("select_pin_origin_unenrolled_refused", CASES[1]),
        ("select_pin_origin_wrong_reason", CASES[1]),
        (CASES[2], CASES[2]),
        (CASES[3], CASES[3]),
    ] {
        let subject = FakeSubject::new(Defects {
            extension: Some(fault),
            ..Defects::default()
        });
        let report = run_suite(&subject).await;
        assert_eq!(report.failing(), [expected], "{}", report.render());
        println!(
            "rejected wrong router ({fault}): {expected}: {:?}",
            report.outcome(expected)
        );
    }
}

#[tokio::test]
async fn select_pin_origin_reserve_cause_is_optional() {
    let subject = FakeSubject::new(Defects {
        extension: Some("select_pin_origin_without_cause"),
        ..Defects::default()
    });
    let report = run_suite(&subject).await;
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    for name in CASES {
        assert!(
            matches!(report.outcome(name), Some(CheckOutcome::Passed)),
            "{name}: {}",
            report.render()
        );
    }
}

#[tokio::test]
async fn select_pin_origin_wrong_cause_is_rejected() {
    let subject = FakeSubject::new(Defects {
        extension: Some("select_pin_origin_wrong_cause"),
        ..Defects::default()
    });
    let report = run_suite(&subject).await;
    assert_eq!(report.failing(), [CASES[1]], "{}", report.render());
    println!(
        "rejected wrong reserve cause: {}: {:?}",
        CASES[1],
        report.outcome(CASES[1])
    );
}
