use cortexkit_role_tool_provider::{call, catalog::CatalogAnswer, errors, withdraw};
use serde_json::{json, Value};
use subc_protocol::ErrorBody;

#[test]
fn fork_calls_catalog_marker_and_role_extension_round_trip() {
    let samples: Value = serde_json::from_str(include_str!(
        "../test-vectors/tool-provider-v1/fork-calls.json"
    ))
    .unwrap();
    let catalog: CatalogAnswer = serde_json::from_value(samples["catalog"].clone()).unwrap();
    assert!(catalog.tools[0].read_only);
    assert!(!catalog.tools[1].read_only);
    for tool in &catalog.tools {
        assert_eq!(
            cortexkit_role_tool_provider::catalog::schema_digest(&tool.input_schema).unwrap(),
            tool.schema_digest
        );
    }
    assert_eq!(serde_json::to_value(catalog).unwrap(), samples["catalog"]);
    let request: call::ToolCallRequest = serde_json::from_value(samples["call"].clone()).unwrap();
    call::check_call(&request).unwrap();
    let id = call::fork_id(&samples["call"]).unwrap().unwrap();
    assert_eq!(call::fork_body(&request, id).unwrap(), samples["call"]);
    call::check_fork_call(&samples["call"], "read", true, false).unwrap();
    for sample in samples["refusals"].as_array().unwrap() {
        let error = call::check_fork_call(
            &sample["call"],
            sample["call"]["name"].as_str().unwrap(),
            sample["read_only"].as_bool().unwrap(),
            sample["would_prompt"].as_bool().unwrap(),
        )
        .unwrap_err();
        assert_eq!(serde_json::to_value(error).unwrap(), sample["error"]);
    }
    let mut unmarked = samples["catalog"]["tools"][1].clone();
    unmarked["future_member"] = json!("ignored by old readers");
    let decoded: cortexkit_role_tool_provider::catalog::CatalogTool =
        serde_json::from_value(unmarked.clone()).unwrap();
    assert!(!decoded.read_only);
    unmarked["read_only"] = json!(false);
    assert!(
        serde_json::from_value::<cortexkit_role_tool_provider::catalog::CatalogTool>(unmarked)
            .is_err()
    );
}

#[test]
fn withdraw_pending_is_retryable_and_delay_is_bounded() {
    let samples: Value = serde_json::from_str(include_str!(
        "../test-vectors/tool-provider-v1/withdraw-pending.json"
    ))
    .unwrap();
    for sample in samples["samples"].as_array().unwrap() {
        let error: ErrorBody = serde_json::from_value(sample["error"].clone()).unwrap();
        assert!(errors::is_transient(&error.code));
        let hint = error
            .detail
            .as_ref()
            .and_then(|d| d["retry_after_ms"].as_u64());
        assert_eq!(
            serde_json::to_value(errors::withdraw_pending(hint)).unwrap(),
            sample["error"]
        );
        assert_eq!(
            errors::withdraw_retry_delay_ms(&error, sample["backoff_ms"].as_u64().unwrap()),
            sample["wait_ms"].as_u64().unwrap()
        );
        assert!(withdraw::caller_verdict(Err(&error)).retries());
    }
}
