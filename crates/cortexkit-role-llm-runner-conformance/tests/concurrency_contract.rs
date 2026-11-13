use cortexkit_role_llm_runner::{
    awaits::{AwaitParty, RunAwaits},
    errors,
    run::{RunError, RunReason, RunStatus},
};
use serde_json::json;

#[test]
fn concurrency_wait_and_account_wait_truth_tables() {
    for code in errors::CODES
        .iter()
        .chain(errors::provider_codes::CODES.iter())
        .copied()
        .chain(["concurrency_full", "future_code", ""])
    {
        let account_wait = matches!(
            code,
            "account_selection_refused" | "account_limit_exhausted"
        );
        let wait = account_wait || code == "concurrency_full";
        assert_eq!(errors::is_account_wait(code), account_wait, "{code}");
        assert_eq!(errors::is_wait(code), wait, "{code}");
        if wait {
            assert!(!errors::is_retryable(code), "{code}");
        }
    }
}

#[test]
fn concurrency_full_is_known_at_admission_and_in_run_errors() {
    assert!(errors::CODES.contains(&"concurrency_full"));
    assert!(errors::provider_codes::CODES.contains(&"concurrency_full"));
    let error = RunError::new("permanent", "capacity occupied", "concurrency_full")
        .with_resets_at_ms(30_000);
    assert_eq!(errors::expected_party(&error), AwaitParty::Caller);
    assert_eq!(errors::ending_awaits(&error, 1_000).next_ms, Some(30_000));
}

#[test]
fn concurrency_full_awaits_sample_round_trips() {
    let wire = json!({
        "state": "paused", "reason": "concurrency_full",
        "awaits": {
            "party": "runner", "reason": "concurrency_full",
            "since_ms": 1_000, "next_ms": 30_000
        }
    });
    let status: RunStatus = serde_json::from_value(wire.clone()).unwrap();
    assert!(!matches!(status.reason, Some(RunReason::Unknown(_))));
    assert_eq!(status.awaits.as_ref().unwrap().party, AwaitParty::Runner);
    assert!(status.awaits_consistent());
    assert_eq!(serde_json::to_value(status).unwrap(), wire);
    let wait: RunAwaits = serde_json::from_value(wire["awaits"].clone()).unwrap();
    assert!(!matches!(wait.reason, Some(RunReason::Unknown(_))));
    assert_eq!(wait.next_ms, Some(30_000));
}
