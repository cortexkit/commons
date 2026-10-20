//! These tests check the kit against a fake, not a live inbox implementation.

mod fake;

use cortexkit_role_agent_inbox_conformance::{run, CaseOutcome, SuiteVerdict, CASES};
use fake::{Defect, Fake};

#[test]
fn conforming_fake_passes_every_case() {
    let report = run(&mut Fake::default());
    assert_eq!(report.verdict, SuiteVerdict::Passed, "{}", report.render());
    assert_eq!(report.cases.len(), CASES.len());
    assert!(report
        .cases
        .iter()
        .all(|case| case.outcome == CaseOutcome::Passed));
    assert_eq!(report.outcome("unknown"), None);
}

macro_rules! detects {
    ($($test:ident => $defect:ident, $case:literal);+ $(;)?) => {$ (
        #[test]
        fn $test() {
            let mut fake = Fake::default();
            fake.defect = Some(Defect::$defect);
            let report = run(&mut fake);
            assert!(matches!(report.outcome($case), Some(CaseOutcome::Failed { .. })), "{}", report.render());
            let control = if $case == "scope_unknown" { "scope_not_yours" } else { "scope_unknown" };
            assert_eq!(report.outcome(control), Some(&CaseOutcome::Passed), "{}", report.render());
            assert!(matches!(report.verdict, SuiteVerdict::Failed { .. }));
        }
    )+};
}

detects! {
    detects_repeat => Repeat, "repeat_first_reply";
    detects_conflict => Conflict, "conflicting_repeat";
    detects_retry_auth => RetryAuth, "retry_before_authorization";
    detects_replace_live => ReplaceLive, "replace_live_attention_and_timing";
    detects_replace_landed => ReplaceLanded, "replace_delivered_untouched";
    detects_replace_retired => ReplaceRetired, "replace_retired_attention";
    detects_expiry => Expiry, "expiry_drops";
    detects_not_before => NotBefore, "deliver_after_holds";
    detects_withdraw => Withdraw, "withdraw_withdrawn";
    detects_withdraw_landed => WithdrawLanded, "withdraw_already_delivered";
    detects_withdraw_missing => WithdrawMissing, "withdraw_not_found";
    detects_namespace => Namespace, "namespace_isolation";
    detects_not_yours => NotYours, "scope_not_yours";
    detects_unknown => Unknown, "scope_unknown";
    detects_ended => Ended, "scope_ended";
    detects_neutralization => Neutralize, "neutralization_delivered";
    detects_cap => Cap, "content_cap";
    detects_pull => Pull, "pull_recovers";
    detects_full => Full, "inbox_full_retry";
}
