//! Plan facts are forwarded unchanged only to providers that opt in.

use cortexkit_role_llm_runner::{
    errors, ops, send::SendReply, system_text_facts::SYSTEM_TEXT_FACTS_V1,
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
    if name == "system_text_host_facts_describe" {
        case.checked_describe().await?;
        return Ok(Ending::Passed);
    }
    let session = case.open(name, Script::default()).await?;
    let declared = name != "system_text_facts_legacy_omitted";
    let malformed = name == "system_text_facts_malformed_refused";
    let provides = if declared {
        vec!["tool-provider/v1".into(), SYSTEM_TEXT_FACTS_V1.into()]
    } else {
        vec!["tool-provider/v1".into(), "system-text-facts/v2".into()]
    };
    let facts = json!({"model":{"provider":"vendor","id":"model"},"harness":{"id":"host","facts":["files","shell"]},"tools":["read","write","é"],"idle_start":["sender.a","sender.b"]});
    let mut item = json!({"preset":"default","params":{"exact":"  keep\n"},"facts":facts});
    if malformed {
        item["facts"]["tools"] = json!(["z", "a"]);
    }
    let expected = serde_json::to_vec(&item["facts"]).map_err(|e| e.to_string())?;
    let fields = case
        .subject
        .prepare_system_text_facts(&session.name, &provides, &item)
        .await
        .map_err(|e| e.to_string())?;
    let before = case
        .subject
        .system_text_default_observation(case.handle.inner(), &session.name)
        .await
        .map_err(|e| e.to_string())?;
    if before.provider_calls != 0 || !before.provider_facts.is_empty() {
        return Err("fixture fetched before admission".into());
    }
    let mut params = Value::Object(fields);
    params["prompt"] = json!("admit system text facts");
    params["send_id"] = json!(case.mint.next(name));
    let reply = call(&session.route, ops::SESSION_SEND, params).await?;
    if malformed {
        let error = expect_error("malformed system-text facts", reply)?;
        expect_code(
            "malformed system-text facts",
            &error,
            errors::INVALID_PARAMS,
        )?;
    } else {
        let _: SendReply = decode(
            "system-text facts admission",
            expect_ok("system-text facts admission", reply)?,
        )?;
    }
    let after = case
        .subject
        .system_text_default_observation(case.handle.inner(), &session.name)
        .await
        .map_err(|e| e.to_string())?;
    if malformed {
        if after.provider_calls != 0
            || !after.provider_facts.is_empty()
            || after.durable_state != before.durable_state
        {
            return Err("malformed facts fetched or wrote state".into());
        }
    } else if after.provider_calls == 0
        || after.provider_facts.len() != after.provider_calls
        || after.provider_facts.iter().any(|received| {
            if declared {
                received.as_ref() != Some(&expected)
            } else {
                received.is_some()
            }
        })
    {
        return Err(format!(
            "provider facts did not match opt-in and exact plan bytes: {:?}",
            after.provider_facts
        ));
    }
    Ok(Ending::Passed)
}
