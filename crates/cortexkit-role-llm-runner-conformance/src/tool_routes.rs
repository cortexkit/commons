//! Live admission, model-definition and dispatch probes for per-item routing.

use cortexkit_role_llm_runner::{capabilities, errors, ops, send::SendReply};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_error, expect_ok, wait_run_end},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};

const TARGET: &str = "conformance-route-target";

fn routes() -> Value {
    json!({"argument":"machine", "description":"Computer to read from", "values":[{"value":"remote", "provider":TARGET}]})
}

fn check_refusal(
    name: &str,
    code: &str,
    detail: Option<&Value>,
    before: &[u8],
    after: &[u8],
) -> Result<(), String> {
    if code != errors::INVALID_PARAMS
        || detail.and_then(|d| d["field"].as_str()) != Some("plan.tool_items[0].routes")
        || before != after
    {
        return Err(format!(
            "{name}: expected indexed invalid_params with no durable write"
        ));
    }
    Ok(())
}

fn check_schema(name: &str, schema: &Value) -> Result<(), String> {
    if schema
        != &json!({"type":"object", "properties":{"marker":{"type":"string"},"machine":{"type":"string","enum":["remote"],"description":"Computer to read from"}},"required":["marker"]})
    {
        return Err(format!(
            "{name}: offered schema did not add exactly the optional typed enum"
        ));
    }
    Ok(())
}

fn check_dispatch(
    name: &str,
    calls: &[(String, Value)],
    arguments: &Value,
    pin: &str,
) -> Result<(), String> {
    if calls.len() != 1
        || calls[0].0 != TARGET
        || calls[0].1["arguments"] != *arguments
        || calls[0].1["schema_pin"] != pin
    {
        return Err(format!("{name}: dispatch must reach the target once with stripped arguments and the frozen default pin"));
    }
    Ok(())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let declared = case
        .checked_describe()
        .await?
        .declares(capabilities::TOOL_ROUTES);
    let admission = name == "tool_routes_admission_refusals";
    if !declared {
        return Ok(Ending::Inapplicable(
            "runner does not declare tool_routes".into(),
        ));
    }
    if admission {
        let samples: Value = serde_json::from_str(
            cortexkit_role_llm_runner::vectors::file("tool-routes.json")
                .ok_or("missing tool routes vectors")?,
        )
        .map_err(|e| e.to_string())?;
        let mut variants = Vec::new();
        for sample in samples["refused"]
            .as_array()
            .ok_or("missing refusal samples")?
        {
            let mut member = routes();
            if let Some(patch) = sample["patch"].as_object() {
                member.as_object_mut().unwrap().extend(patch.clone());
            }
            if let Some(remove) = sample["remove"].as_str() {
                member.as_object_mut().unwrap().remove(remove);
            }
            variants.push((member, false));
        }
        variants.push((routes(), true));
        for (member, clash) in variants {
            let session = case.open(name, Script { turns: vec![] }).await?;
            let Some(mut fields) =
                case.subject
                    .tool_routes_plan_fields(&session.name, &member, clash)
            else {
                return Ok(Ending::Inapplicable(
                    "adapter has no tool_routes plan fixture".into(),
                ));
            };
            verify_fixture(&fields, &member)?;
            let before = case
                .subject
                .tool_routes_durable_state(case.handle.inner(), &session.name)
                .await
                .map_err(|e| e.to_string())?;
            fields.insert("prompt".into(), json!("route admission"));
            fields.insert("send_id".into(), json!(case.mint.next(name)));
            let error = expect_error(
                name,
                call(&session.route, ops::SESSION_SEND, Value::Object(fields)).await?,
            )?;
            let after = case
                .subject
                .tool_routes_durable_state(case.handle.inner(), &session.name)
                .await
                .map_err(|e| e.to_string())?;
            check_refusal(name, &error.code, error.detail.as_ref(), &before, &after)?;
        }
    } else {
        let mut tool = case.mint.call("routed-call");
        let stripped = tool.arguments.clone();
        tool.arguments["machine"] = json!("remote");
        let session = case
            .open(
                name,
                Script {
                    turns: vec![
                        ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool)]),
                        ScriptedTurn::new(vec![case.mint.text(name).0]),
                    ],
                },
            )
            .await?;
        let member = routes();
        let Some(mut fields) = case
            .subject
            .tool_routes_plan_fields(&session.name, &member, false)
        else {
            return Ok(Ending::Inapplicable(
                "adapter has no tool_routes plan fixture".into(),
            ));
        };
        verify_fixture(&fields, &member)?;
        fields.insert("prompt".into(), json!("read on remote"));
        fields.insert("send_id".into(), json!(case.mint.next(name)));
        let reply: SendReply = decode(
            name,
            expect_ok(
                name,
                call(&session.route, ops::SESSION_SEND, Value::Object(fields)).await?,
            )?,
        )?;
        wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
        match name {
            "tool_routes_offered_definition" => check_schema(
                name,
                &case
                    .subject
                    .tool_routes_offered_schema(&session.name)
                    .await
                    .map_err(|e| e.to_string())?,
            )?,
            "tool_routes_strips_argument" => {
                let pin = case
                    .subject
                    .tool_routes_schema_pin()
                    .ok_or("tool_routes fixture omitted its default schema pin")?;
                let calls = case
                    .subject
                    .tool_routes_calls(&session.name)
                    .await
                    .map_err(|e| e.to_string())?;
                check_dispatch(name, &calls, &stripped, &pin)?;
            }
            _ => return Err(format!("unknown tool routes check {name}")),
        }
    }
    Ok(Ending::Passed)
}

fn verify_fixture(fields: &serde_json::Map<String, Value>, member: &Value) -> Result<(), String> {
    let items = fields
        .get("plan")
        .and_then(|p| p["tool_items"].as_array())
        .ok_or("tool routes fixture needs tool_items")?;
    if items.len() != 1 || items[0]["routes"] != *member {
        return Err(
            "tool routes fixture must preserve the supplied member on its single item".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_routes_bad_implementations_fail_the_named_checks() {
        let name = "tool_routes_admission_refusals";
        let detail = json!({"field":"plan.tool_items[0].routes"});
        check_refusal(name, "invalid_params", Some(&detail), b"before", b"before").unwrap();
        for (code, detail, after) in [
            ("ok", Some(&detail), b"before".as_slice()),
            ("invalid_params", None, b"before".as_slice()),
            ("invalid_params", Some(&detail), b"written".as_slice()),
        ] {
            assert!(check_refusal(name, code, detail, b"before", after)
                .unwrap_err()
                .starts_with(name));
        }
        let name = "tool_routes_offered_definition";
        let good = json!({"type":"object", "properties":{"marker":{"type":"string"},"machine":{"type":"string","enum":["remote"],"description":"Computer to read from"}},"required":["marker"]});
        check_schema(name, &good).unwrap();
        let mut bad = good.clone();
        bad["properties"]["machine"]
            .as_object_mut()
            .unwrap()
            .remove("type");
        assert!(check_schema(name, &bad).unwrap_err().starts_with(name));
        let mut bad = good;
        bad["required"] = json!(["marker", "machine"]);
        assert!(check_schema(name, &bad).unwrap_err().starts_with(name));
        let name = "tool_routes_strips_argument";
        let good = vec![(
            TARGET.into(),
            json!({"arguments":{"marker":"read"}, "schema_pin":"frozen-default"}),
        )];
        let expected = json!({"marker":"read"});
        check_dispatch(name, &good, &expected, "frozen-default").unwrap();
        for bad in [
            vec![("default".into(), good[0].1.clone())],
            vec![(
                TARGET.into(),
                json!({"arguments":{"marker":"read","machine":"remote"},"schema_pin":"frozen-default"}),
            )],
            vec![(
                TARGET.into(),
                json!({"arguments":{"marker":"read"},"schema_pin":"target-new-pin"}),
            )],
            vec![good[0].clone(), good[0].clone()],
        ] {
            assert!(check_dispatch(name, &bad, &expected, "frozen-default")
                .unwrap_err()
                .starts_with(name));
        }
    }
}
