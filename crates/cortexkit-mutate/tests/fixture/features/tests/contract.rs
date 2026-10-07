use feature_fixture::test_support::guarded;

#[test]
fn feature_gated_guard() {
    use std::io::Write;
    writeln!(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(".git/guard-runs")
            .unwrap(),
        "run"
    )
    .unwrap();
    assert!(!guarded(0));
}

#[test]
fn feature_baseline() {
    assert!(!cfg!(feature = "strict-baseline"));
}
