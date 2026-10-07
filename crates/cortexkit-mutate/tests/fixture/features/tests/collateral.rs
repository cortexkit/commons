use feature_fixture::test_support::guarded;

#[test]
fn collateral_guard() {
    assert!(!guarded(0));
}
