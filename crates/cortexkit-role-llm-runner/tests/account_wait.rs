use cortexkit_role_llm_runner::{errors, read::LastRunError, run::RunError};
use serde_json::json;

#[test]
fn account_selection_final_is_known_but_never_waits_or_retries() {
    let code = "account_selection_final";
    assert_eq!(errors::ACCOUNT_SELECTION_FINAL, code);
    assert_eq!(errors::provider_codes::ACCOUNT_SELECTION_FINAL, code);
    assert!(errors::CODES.contains(&code));
    assert!(errors::provider_codes::CODES.contains(&code));
    assert!(!errors::is_retryable(code));
    assert!(!errors::is_account_wait(code));
}

#[test]
fn run_error_provider_code_decides_account_wait() {
    for (code, waits) in [
        ("account_limit_exhausted", true),
        ("account_selection_refused", true),
        ("account_selection_final", false),
        ("invalid_request", false),
        ("future_code", false),
    ] {
        assert!(
            !waits || errors::provider_codes::CODES.contains(&code),
            "missing run provider code: {code}"
        );
        let error = RunError::new("permanent", "account unavailable", code);
        assert_eq!(errors::is_account_wait(&error.provider_code), waits);
        assert!(!errors::is_retryable(&error.provider_code));
    }
}

#[test]
fn account_wait_optional_members_decode_and_round_trip() {
    let run_wire = json!({
        "class": "transient",
        "message": "account unavailable",
        "provider_code": "account_selection_refused",
        "retry_after_secs": 1.5,
        "resets_at_ms": 1234,
        "routing_code": "store_error"
    });
    let run: RunError = serde_json::from_value(run_wire.clone()).unwrap();
    assert_eq!(run.routing_code.as_deref(), Some("store_error"));
    assert_eq!(serde_json::to_value(&run).unwrap(), run_wire);
    assert_eq!(
        run,
        RunError::new(
            "transient",
            "account unavailable",
            "account_selection_refused"
        )
        .with_retry_after_secs(1.5)
        .with_resets_at_ms(1234)
        .with_routing_code("store_error")
    );

    let head_wire = json!({
        "class": "transient",
        "retry_after_secs": 1.5,
        "resets_at_ms": 1234,
        "provider_code": "account_selection_refused"
    });
    let head: LastRunError = serde_json::from_value(head_wire.clone()).unwrap();
    assert_eq!(
        head.provider_code.as_deref(),
        Some("account_selection_refused")
    );
    assert_eq!(serde_json::to_value(&head).unwrap(), head_wire);
    assert_eq!(
        head,
        LastRunError::new("transient")
            .with_retry_after_secs(
                cortexkit_role_llm_runner::read::RetryAfterSecs::new(1.5).unwrap()
            )
            .with_resets_at_ms(1234)
            .with_provider_code("account_selection_refused")
    );
}

#[test]
fn account_wait_absent_members_keep_existing_bytes() {
    let run_wire = r#"{"class":"transient","message":"account unavailable","provider_code":"account_limit_exhausted","retry_after_secs":1.5,"resets_at_ms":1234}"#;
    let run: RunError = serde_json::from_str(run_wire).unwrap();
    assert_eq!(run.routing_code, None);
    assert_eq!(serde_json::to_string(&run).unwrap(), run_wire);
    assert_eq!(
        serde_json::to_string(&RunError::new(
            "permanent",
            "no account",
            "account_selection_final"
        ))
        .unwrap(),
        r#"{"class":"permanent","message":"no account","provider_code":"account_selection_final"}"#
    );

    let head_wire = r#"{"class":"transient","retry_after_secs":1.5,"resets_at_ms":1234}"#;
    let head: LastRunError = serde_json::from_str(head_wire).unwrap();
    assert_eq!(head.provider_code, None);
    assert_eq!(serde_json::to_string(&head).unwrap(), head_wire);
    assert_eq!(
        serde_json::to_string(&LastRunError::new("transient")).unwrap(),
        r#"{"class":"transient"}"#
    );
}

#[test]
fn account_wait_error_shapes_ignore_future_members() {
    let run: RunError = serde_json::from_value(json!({
        "class": "transient",
        "message": "account unavailable",
        "provider_code": "account_selection_refused",
        "future_member": true
    }))
    .unwrap();
    assert_eq!(run.provider_code, "account_selection_refused");
    let head: LastRunError = serde_json::from_value(json!({
        "class": "transient",
        "future_member": true
    }))
    .unwrap();
    assert_eq!(head.class, "transient");
}
