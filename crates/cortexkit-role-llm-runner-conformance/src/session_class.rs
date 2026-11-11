//! Session-label syntax and freezing, driven through ordinary session sends.

use cortexkit_role_llm_runner::{capabilities, describe::RoleDescribe, ops, send::SendReply};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_code, expect_error, expect_ok, wait_run_end},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

fn params<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    id: &str,
    label: Option<Value>,
) -> Value
where
    S::Route: RunnerRoute,
{
    let mut params = case.params(session, "class prompt", id, None);
    params.as_object_mut().unwrap().remove("session_class");
    if let Some(label) = label {
        params["session_class"] = label;
    }
    params
}

async fn accept<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    label: Option<Value>,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let id = case.mint.next("class-send");
    let reply: SendReply = decode(
        "session_class send",
        expect_ok(
            "session_class send",
            call(
                &session.route,
                ops::SESSION_SEND,
                params(case, session, &id, label),
            )
            .await?,
        )?,
    )?;
    session.first.get_or_insert((id, "class prompt".into()));
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    Ok(())
}

async fn refuse<R: RunnerRoute>(route: &R, params: Value) -> Result<(), String> {
    let error = expect_error(
        "session_class refusal",
        call(route, ops::SESSION_SEND, params).await?,
    )?;
    expect_code("session_class refusal", &error, "invalid_params")?;
    if error
        .detail
        .as_ref()
        .and_then(|detail| detail.get("field"))
        .and_then(Value::as_str)
        != Some("session_class")
    {
        return Err(format!(
            "expected invalid_params naming session_class, got {error:?}"
        ));
    }
    Ok(())
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "session-class",
            Script {
                turns: (0..4)
                    .map(|_| ScriptedTurn::new(vec![ScriptedPart::Text("class-answer".into())]))
                    .collect(),
            },
        )
        .await?;
    let describe: RoleDescribe = decode(
        "describe",
        expect_ok(
            "describe",
            call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
        )?,
    )?;
    if !describe.declares(capabilities::SESSION_CLASS) {
        return Ok(Ending::Inapplicable("session_class is not declared".into()));
    }
    match name {
        "session_class_malformed" => {
            for label in [
                json!(null),
                json!(17),
                json!(true),
                json!([]),
                json!({}),
                json!(""),
                json!("Upper"),
                json!("has space"),
                json!("has.dot"),
                json!("é"),
                json!("a".repeat(33)),
            ] {
                let id = case.mint.next("class-invalid");
                refuse(&session.route, params(case, &session, &id, Some(label))).await?;
            }
        }
        "session_class_frozen" => {
            accept(case, &mut session, Some(json!("class-a"))).await?;
            let id = case.mint.next("class-conflict");
            refuse(
                &session.route,
                params(case, &session, &id, Some(json!("class-b"))),
            )
            .await?;
        }
        "session_class_repeat_or_omit" => {
            accept(case, &mut session, Some(json!("class-a"))).await?;
            accept(case, &mut session, Some(json!("class-a"))).await?;
            accept(case, &mut session, None).await?;
        }
        "session_class_unlabelled" => {
            accept(case, &mut session, None).await?;
            accept(case, &mut session, Some(json!("class-a"))).await?;
            accept(case, &mut session, Some(json!("class-a"))).await?;
            accept(case, &mut session, None).await?;
        }
        _ => return Err(format!("unknown session_class case {name}")),
    }
    Ok(Ending::Passed)
}
