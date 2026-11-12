//! Preset-less system text depends on both the runner and provider declarations.

use cortexkit_role_llm_runner::{
    errors, ops, send::SendReply, SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_ok},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script},
};

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case.checked_describe().await?.system_text_default() {
        return Ok(Ending::Inapplicable(
            "runner does not declare system_text_default".into(),
        ));
    }
    let session = case.open(name, Script::default()).await?;
    let advertised = name == "system_text_default_forwarded_unchanged";
    let provides = if advertised {
        vec![
            "tool-provider/v1".into(),
            SYSTEM_TEXT_DEFAULT_PROVIDER_CAPABILITY.into(),
        ]
    } else {
        vec!["tool-provider/v1".into(), "system-text-default/v2".into()]
    };
    let item = json!({"params": {"fixture": "default-text", "untouched": [1, "  exact\n", {"nested": true}]}});
    let expected = serde_json::to_vec(&item).map_err(|e| e.to_string())?;
    let fields = case
        .subject
        .prepare_system_text_default(&session.name, &provides, &item)
        .await
        .map_err(|e| e.to_string())?;
    let before = case
        .subject
        .system_text_default_observation(case.handle.inner(), &session.name)
        .await
        .map_err(|e| e.to_string())?;
    if before.provider_calls != 0 || !before.provider_items.is_empty() {
        return Err("fixture forwarded system text before admission".into());
    }
    let mut params = Value::Object(fields);
    params["prompt"] = json!("admit default system text");
    params["send_id"] = json!(case.mint.next(name));
    let reply = call(&session.route, ops::SESSION_SEND, params).await?;
    let refusal = if advertised {
        let _: SendReply = decode(
            "preset-less system-text admission",
            expect_ok("preset-less system-text admission", reply)?,
        )?;
        None
    } else {
        Some(expect_error("preset-less system-text admission", reply)?)
    };
    let after = case
        .subject
        .system_text_default_observation(case.handle.inner(), &session.name)
        .await
        .map_err(|e| e.to_string())?;
    if advertised {
        if after.provider_calls == 0
            || after.provider_items.is_empty()
            || after.provider_items.iter().any(|item| item != &expected)
        {
            return Err(format!(
                "provider did not receive the preset-less item byte-equal: {:?}",
                after.provider_items
            ));
        }
    } else {
        let refusal = refusal.unwrap();
        expect_code(
            "preset-less system-text admission",
            &refusal,
            errors::SYSTEM_TEXT_PRESET_REQUIRED,
        )?;
        if refusal
            .detail
            .as_ref()
            .and_then(|detail| detail.get("provider"))
            != Some(&json!(case.subject.tool_provider_module()))
        {
            return Err(format!(
                "system_text_preset_required must name the provider: {:?}",
                refusal.detail
            ));
        }
        if after.durable_state != before.durable_state
            || after.provider_calls != 0
            || !after.provider_items.is_empty()
        {
            return Err("refused system-text plan wrote state or forwarded an item".into());
        }
    }
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    #[test]
    fn unknown_refusal_codes_survive_transport_decoding() {
        let bytes = br#"{"code":"future_refusal","message":"future","detail":{"provider":"future.module"}}"#;
        let body: subc_protocol::ErrorBody = serde_json::from_slice(bytes).unwrap();
        assert_eq!(body.code, "future_refusal");
        assert_eq!(body.detail.as_ref().unwrap()["provider"], "future.module");
        assert!(!cortexkit_role_llm_runner::errors::is_retryable(&body.code));
        assert!(!cortexkit_role_llm_runner::errors::is_account_wait(
            &body.code
        ));
        let encoded = serde_json::to_value(&body).unwrap();
        assert_eq!(encoded["code"], "future_refusal");
        assert_eq!(encoded["detail"]["provider"], "future.module");
    }

    #[test]
    fn account_wait_uses_only_the_outer_refusal_code() {
        use cortexkit_role_llm_runner::errors;
        use serde_json::json;

        for (code, account_wait, retryable) in [
            ("account_selection_refused", true, false),
            ("account_limit_exhausted", true, false),
            ("account_selection_final", false, false),
            ("transient", false, true),
            ("fetch_unavailable", false, true),
            ("scope_not_synced", false, true),
            ("session_released", false, true),
            ("fork_busy", false, true),
            ("fork_account_held", false, true),
            ("future_refusal", false, false),
            ("", false, false),
        ] {
            for detail in [
                None,
                Some(json!({"class": "transient", "routing_code": "unknown_refusal"})),
                Some(json!({"class": "permanent", "routing_code": "account_pool_exhausted"})),
                Some(json!({
                    "class": "account_selection_refused",
                    "routing_code": "account_limit_exhausted"
                })),
                Some(json!({
                    "routing_code": "account_pool_exhausted",
                    "class": "transient",
                    "retry_after_s": 60,
                    "retry_after_secs": 60.0,
                    "decision_id": "acd-refused",
                    "provider_code": "account_limit_exhausted",
                    "routing_detail": {
                        "code": "account_pool_exhausted",
                        "retry_after_s": 60,
                        "decision_id": "acd-refused",
                        "exclusions": []
                    },
                    "future_hint": "ignored"
                })),
            ] {
                let mut wire = json!({"code": code, "message": "refused"});
                if let Some(detail) = detail {
                    wire["detail"] = detail;
                }
                let body: subc_protocol::ErrorBody = serde_json::from_value(wire.clone()).unwrap();
                assert_eq!(errors::is_account_wait(&body.code), account_wait, "{wire}");
                assert_eq!(errors::is_retryable(&body.code), retryable, "{wire}");
                assert_eq!(serde_json::to_value(&body).unwrap(), wire);
            }
        }
    }
}
