use ignored_fixture::{bounded, guarded};
use std::io::Write;

fn record(name: &str) {
    writeln!(
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(".git/guard-runs")
            .unwrap(),
        "{name}"
    )
    .unwrap();
}

#[test]
fn ordinary_guard() {
    record("ordinary");
    assert!(!bounded(10));
}

#[test]
#[ignore = "stands in for a guard that needs a real daemon"]
fn ignored_guard() {
    record("ignored");
    assert!(!guarded(0));
}

#[test]
#[ignore]
fn ignored_baseline() {
    panic!("red whenever ignored tests run");
}
