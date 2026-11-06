use super::*;

fn subject(defects: Defects) -> FakeSubject {
    let mut subject = routed_accounts_subject(defects);
    subject.capabilities.insert(Capability::AccountRoute);
    subject
}

#[tokio::test]
async fn account_route_faithful_runner_passes() {
    let report = run(&subject(Defects::default())).await;
    for name in cases_of(&["account_route_"]) {
        assert_passed(&report, name);
    }
    assert_none_failed_but(&report, &[]);
}

#[tokio::test]
async fn account_route_vendor_names_fail_by_name() {
    let report = run(&subject(Defects {
        account_route_vendor_names: true,
        ..Defects::default()
    }))
    .await;
    let name = "account_route_names_the_route";
    let reason = failed(&report, name);
    println!("FAIL {name}: {reason}");
    assert!(reason.contains("account_route"), "{reason}");
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn account_route_changed_join_fails_by_name() {
    let report = run(&subject(Defects {
        account_route_change_accepted: true,
        ..Defects::default()
    }))
    .await;
    let name = "account_route_fixed_per_run";
    let reason = failed(&report, name);
    println!("FAIL {name}: {reason}");
    assert_none_failed_but(&report, &[name]);
}

#[tokio::test]
async fn account_route_undeclared_is_inapplicable() {
    for subject in [
        FakeSubject::new(Defects::default()),
        routed_accounts_subject(Defects::default()),
    ] {
        let report = run(&subject).await;
        for name in cases_of(&["account_route_"]) {
            assert!(
                matches!(report.outcome(name), Some(CaseOutcome::Inapplicable { .. })),
                "{name}: {}",
                report.render()
            );
        }
        assert_none_failed_but(&report, &[]);
    }
}
