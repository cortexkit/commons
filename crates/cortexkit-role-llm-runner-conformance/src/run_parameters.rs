//! Independent discovery-gated run limits and tool suppression (§9.3.1).

use cortexkit_role_llm_runner::{capabilities as groups, errors, ops, send::SendReply};

use crate::subject::Capability;
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_error, expect_field, expect_ok, read_all, run_result, wait_run_end,
    },
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn, ScriptedUsage},
};

async fn send<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    params: Value,
) -> Result<crate::drive::RunEnd, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let id = params["send_id"].as_str().unwrap().to_owned();
    let prompt = params["prompt"].as_str().unwrap().to_owned();
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first.get_or_insert((id, prompt));
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "max_steps_seals_pending_calls" => max_steps(case).await,
        "max_steps_final_step_completes" => final_step(case).await,
        "max_output_tokens_caps_response" => output_cap(case).await,
        "no_tools_offered_to_model" => no_tools(case).await,
        "no_tools_later_plan_send_refused" => later_tools(case).await,
        other => Err(format!("unknown run parameter case {other}")),
    }
}

async fn max_steps<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let prior = case.mint.call("steps-prior");
    let pending = [
        case.mint.call("steps-pending-a"),
        case.mint.call("steps-pending-b"),
    ];
    let mut session = case
        .open(
            "max-steps",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(prior.clone())]),
                    ScriptedTurn::new(
                        pending
                            .iter()
                            .cloned()
                            .map(ScriptedPart::ToolCall)
                            .collect(),
                    ),
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(
                        case.mint.call("steps-too-far"),
                    )]),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::MAX_STEPS, ops::SESSION_SEND).await?;
    let mut params = case.params(
        &session,
        "keep calling tools",
        &case.mint.next("steps"),
        None,
    );
    params["stop_when"]["max_steps"] = json!(2);
    let end = send(case, &mut session, params).await?;
    let result = run_result(&session.route, &end.run_id).await?;
    if result.state != "max_steps" {
        return Err(format!(
            "expected max_steps after two model responses, got {result:?}"
        ));
    }
    // Read immediately after the terminal to check that the pending calls
    // already have results, before any follow-up send could append them.
    let (_, messages) = read_all(&session.route).await?;
    let count = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    if count != 2 || case.subject.tool_invocations(&prior.arguments).await != 1 {
        return Err(format!("expected two model responses and one under-limit tool invocation, got {count} responses"));
    }
    for tool in pending {
        if case.subject.tool_invocations(&tool.arguments).await != 0 {
            return Err(format!(
                "pending call {} was dispatched at max_steps",
                tool.tool_call_id
            ));
        }
        let results: Vec<_> = messages
            .iter()
            .filter_map(|m| {
                case.subject
                    .not_executed_tool_result(&m.message, &tool.tool_call_id)
            })
            .collect();
        if results.len() != 1 || results[0].executed {
            return Err(format!("terminal max_steps exposed without exactly one never-executed result for {}: {results:?}", tool.tool_call_id));
        }
    }
    Ok(Ending::Passed)
}

/// A step that reaches the limit without asking for a tool call ends the
/// run `completed`, with its final message (and its directives, where the
/// runner declares `turn_directives`): the limit cut nothing off.
async fn final_step<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let answer = case.mint.next("limit-answer");
    // Not the 90-second wake the directive cases use, so a runner failing
    // one of those fails only that case.
    let wake = "<wake in=\"2m\"/>";
    let directives = case.declared.contains(&Capability::TurnDirectives);
    let mut parts = vec![ScriptedPart::Text(format!("{answer} "))];
    if directives {
        parts.push(ScriptedPart::Text(wake.into()));
    }
    let mut session = case
        .open(
            "max-steps-final",
            Script {
                turns: vec![
                    ScriptedTurn::new(parts),
                    ScriptedTurn::new(vec![ScriptedPart::Text("must not start".into())]),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::MAX_STEPS, ops::SESSION_SEND).await?;
    let mut params = case.params(
        &session,
        "answer in one step",
        &case.mint.next("steps-final"),
        None,
    );
    params["stop_when"]["max_steps"] = json!(1);
    if directives {
        params["directives"] = json!(["wake"]);
    }
    let end = send(case, &mut session, params).await?;
    let result = run_result(&session.route, &end.run_id).await?;
    if result.state != "completed" {
        return Err(format!(
            "a step that reached max_steps: 1 with no tool call must end the run completed, \
             got {}: {result:?}",
            result.state
        ));
    }
    let expected = if directives {
        format!("{answer} {wake}")
    } else {
        format!("{answer} ")
    };
    if result
        .final_message
        .as_ref()
        .is_none_or(|message| message.text != expected)
    {
        return Err(format!(
            "the completed run's final_message is not the limiting step's text {expected:?}: \
             {:?}",
            result.final_message
        ));
    }
    if directives {
        let reported = serde_json::to_value(&result.directives).unwrap();
        let wanted = json!([{"name":"wake","in_ms":120000}]);
        if reported != wanted {
            return Err(format!(
                "the completed run reports directives {reported}, not {wanted}"
            ));
        }
    }
    Ok(Ending::Passed)
}

async fn output_cap<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for requested in [3, u64::MAX] {
        let mut session = case
            .open(
                "output-cap",
                Script {
                    turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                        "one two three four five six seven".into(),
                    )])
                    .with_usage(ScriptedUsage::new(1, 0, 0, 7))],
                },
            )
            .await?;
        crate::tags::require_declared(&session.route, groups::MAX_OUTPUT_TOKENS, ops::SESSION_SEND)
            .await?;
        let mut params = case.params(
            &session,
            "bounded response",
            &case.mint.next("output-cap"),
            None,
        );
        params["generation"]["max_output_tokens"] = json!(requested);
        let end = send(case, &mut session, params).await?;
        let result = run_result(&session.route, &end.run_id).await?;
        let output = result
            .usage
            .as_ref()
            .and_then(|usage| usage.output_tokens)
            .ok_or_else(|| format!("single response has no reported output usage: {result:?}"))?;
        let requests = case
            .subject
            .model_requests(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        if result.state != "completed"
            || result.usage.as_ref().is_none_or(|usage| usage.steps != 1)
            || requests.len() != 1
            || requests[0]
                .max_output_tokens
                .is_none_or(|cap| cap == 0 || cap > requested)
            || output == 0
            || output > requested
        {
            return Err(format!("response exceeds cap {requested} or no positive cap reached model: output_tokens={output}, requests={requests:?}, result={result:?}"));
        }
    }
    Ok(Ending::Passed)
}

async fn no_tools<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    // The unsuppressed control proves the adapter normally offers tools, so
    // observing an empty request cannot pass merely because it always omits them.
    for suppress in [false, true] {
        let mut session = case
            .open(
                "no-tools",
                Script {
                    turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text("done".into())])],
                },
            )
            .await?;
        crate::tags::require_declared(&session.route, groups::NO_TOOLS, ops::SESSION_SEND).await?;
        let mut params = case.params(
            &session,
            "tool-free response",
            &case.mint.next("no-tools"),
            None,
        );
        if suppress {
            params["tools"] = json!([]);
        }
        send(case, &mut session, params).await?;
        let requests = case
            .subject
            .model_requests(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        if requests.len() != 1 || requests[0].tools.is_empty() != suppress {
            return Err(format!(
                "tools [] suppression={suppress}, but model received {requests:?}"
            ));
        }
    }
    Ok(Ending::Passed)
}

async fn later_tools<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "late-plan-tools",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::Text("first".into())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("must not start".into())]),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::NO_TOOLS, ops::SESSION_SEND).await?;
    let mut params = case.params(&session, "freeze plan", &case.mint.next("plan-tools"), None);
    if !params.get("plan").is_some_and(Value::is_object) {
        let Some(fields) =
            case.subject
                .send_fields_with_plan(&session.name, true, &Default::default())
        else {
            return Ok(Ending::Inapplicable(
                "the adapter cannot create a fetch-plan session".into(),
            ));
        };
        params.as_object_mut().unwrap().extend(fields);
        if !params.get("plan").is_some_and(Value::is_object) {
            return Ok(Ending::Inapplicable(
                "the adapter supplies no fetch plan".into(),
            ));
        }
    }
    send(case, &mut session, params).await?;
    let before = read_all(&session.route).await?.1;
    for tools in [json!([]), json!([crate::SCRIPTED_TOOL])] {
        let mut params = case.params(
            &session,
            "change tools",
            &case.mint.next("late-tools"),
            None,
        );
        params["tools"] = tools;
        let error = expect_error(
            "late plan tools",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?;
        expect_field("late plan tools", &error, errors::INVALID_PARAMS, "tools")?;
    }
    if read_all(&session.route).await?.1 != before {
        return Err("refused late tools wrote messages".into());
    }
    Ok(Ending::Passed)
}
