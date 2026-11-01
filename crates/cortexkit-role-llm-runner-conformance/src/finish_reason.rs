//! Discovery-gated normalized finishes, observed only over management routes.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    finish::{self, Finish},
    ops,
};
use serde_json::json;

use crate::{
    cases::{Case, Ending},
    drive::{call, expect_ok, run_result},
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn, ScriptedUsage},
};

pub(crate) fn raw_stop(source: &str) -> Result<&'static str, String> {
    match source {
        "anthropic.messages" | "aws.bedrock_converse" => Ok("end_turn"),
        "openai.chat_completions" | "ai-sdk" => Ok("stop"),
        "openai.responses" | "openai.chatgpt_responses" => Ok("completed"),
        "google.generate_content" | "google.code_assist" => Ok("STOP"),
        other => Err(format!(
            "adapter supplied unknown finish reason source {other}"
        )),
    }
}

pub(crate) async fn check<S>(
    case: &Case<'_, S>,
    route: &S::Route,
    run_id: &str,
    expected: Option<Finish>,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let expected = expected.as_ref().map(|finish| json!(finish.as_str()));
    for op in [ops::RUN_RESULT, ops::RUN_STATUS] {
        if op == ops::RUN_STATUS && !case.declared.contains(&Capability::RunStatus) {
            continue;
        }
        let body = expect_ok(op, call(route, op, json!({"run_id":run_id})).await?)?;
        if body.get("finish") != expected.as_ref() {
            return Err(format!(
                "{op} finish {:?}, expected {expected:?} for run {run_id}",
                body.get("finish")
            ));
        }
    }
    Ok(())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if name == "finish_reason_no_model_step" {
        return no_model_step(case).await;
    }
    let Some(source) = case.subject.model_finish_reason_source() else {
        return Ok(Ending::Inapplicable(
            "adapter cannot script raw model finish reasons".into(),
        ));
    };
    let raw = raw_stop(source)?;
    if name == "finish_reason_output_cap" {
        return crate::run_parameters::output_cap(case, true).await;
    }
    if name != "finish_reason_last_model_step" {
        return Err(format!("unknown finish reason case {name}"));
    }
    let mut session = case
        .open(
            "finish-last-step",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(case.mint.call("finish-tool"))])
                        .with_finish_reason(raw),
                    ScriptedTurn::new(vec![ScriptedPart::Text("the final answer".into())])
                        .with_finish_reason(raw),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::FINISH_REASON, ops::RUN_RESULT).await?;
    let params = case.params(
        &session,
        "use a tool then answer",
        &case.mint.next("finish-send"),
        None,
    );
    let end = crate::run_parameters::send(case, &mut session, params).await?;
    let count = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|error| error.to_string())?;
    if count != 2 {
        return Err(format!("expected two model steps, got {count}"));
    }
    check(
        case,
        &session.route,
        &end.run_id,
        finish::normalize(source, Some(raw), false).map_err(|error| error.to_string())?,
    )
    .await?;
    Ok(Ending::Passed)
}

async fn no_model_step<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut warmup = ScriptedTurn::new(vec![ScriptedPart::Text("one token".into())])
        .with_usage(ScriptedUsage::new(1, 0, 0, 2));
    if let Some(source) = case.subject.model_finish_reason_source() {
        warmup = warmup.with_finish_reason(raw_stop(source)?);
    }
    let mut session = case
        .open(
            "finish-no-step",
            Script {
                turns: vec![
                    warmup,
                    ScriptedTurn::new(vec![ScriptedPart::Text("must not run".into())]),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::FINISH_REASON, ops::RUN_RESULT).await?;
    let mut params = case.params(
        &session,
        "consume the input budget",
        &case.mint.next("finish-warmup"),
        None,
    );
    params["budget"] = json!({"max_input_tokens":1});
    crate::run_parameters::send(case, &mut session, params).await?;
    let before = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|error| error.to_string())?;
    let params = case.params(
        &session,
        "budget already spent",
        &case.mint.next("finish-no-step"),
        None,
    );
    let end = crate::run_parameters::send(case, &mut session, params).await?;
    let after = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|error| error.to_string())?;
    let result = run_result(&session.route, &end.run_id).await?;
    if before != 1 || after != before || result.state != "budget_exhausted" {
        return Err(format!("expected no model step after budget exhaustion: before={before}, after={after}, result={result:?}"));
    }
    check(case, &session.route, &end.run_id, None).await?;
    Ok(Ending::Passed)
}
