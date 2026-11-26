//! Check active step liveness and output bounds at the runner's public boundary.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    errors::{provider_codes, tool_result_reasons},
    finish::Finish,
    ops,
    run::{RunState, RunStatus, RunStatusStepPhase},
    tool_calls::outcomes,
};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, run_result},
    finish_reason,
    route::RunnerRoute,
    send_helpers::{require_declared, send},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

pub(crate) async fn liveness<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, _) = case.mint.text("after-hold");
    let mut session = case
        .open(
            "quiet-step",
            Script {
                turns: vec![ScriptedTurn::new(vec![answer])],
            },
        )
        .await?;
    require_declared(&session.route, groups::STEP_LIVENESS, ops::RUN_STATUS).await?;
    require_declared(&session.route, groups::RUN_STATUS, ops::RUN_STATUS).await?;
    case.subject
        .hold_next_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let check = async {
        let reply = send(
            case,
            &mut session,
            None,
            "answer after the hold",
            &Map::new(),
        )
        .await?;
        case.subject
            .await_model_step(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        let run_id = match reply.run_id {
            Some(id) => id,
            None => {
                head(&session.route)
                    .await?
                    .last_run_state
                    .ok_or("the held send has no observable run")?
                    .run_id
            }
        };
        let body = expect_ok(
            ops::RUN_STATUS,
            call(&session.route, ops::RUN_STATUS, json!({"run_id": run_id})).await?,
        )?;
        let status: RunStatus = decode(ops::RUN_STATUS, body)?;
        if status.run_state() != RunState::Active {
            return Err(format!(
                "expected active run while the model response is held: {status:?}"
            ));
        }
        let step = status.step.ok_or("active run has no run.status step")?;
        if step.phase != RunStatusStepPhase::AwaitingResponse
            || step.idle_deadline_ms.is_none()
            || step.content_deadline_ms.is_none()
            || step.last_event_at_ms.is_some()
        {
            return Err(format!(
                "expected awaiting_response with both deadlines and no last event: {step:?}"
            ));
        }
        Ok(Ending::Passed)
    }
    .await;
    let release = case.subject.release_model_step(&session.name).await;
    let verdict = check?;
    release.map_err(|e| e.to_string())?;
    Ok(verdict)
}

pub(crate) async fn cut_calls<S>(case: &Case<'_, S>, three: bool) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case.subject.supports_tool_argument_prefixes() {
        return Ok(Ending::Inapplicable(
            "adapter cannot stream raw tool argument prefixes".into(),
        ));
    }
    let source = case
        .subject
        .model_finish_reason_source()
        .ok_or("tool argument prefix fixture needs a model finish reason source")?;
    let raw_length = finish_reason::raw_length(source)?;
    let calls: Vec<_> = (0..if three { 3 } else { 1 })
        .map(|index| case.mint.call(&format!("cut-call-{index}")))
        .collect();
    let sibling = (!three).then(|| case.mint.call("complete-sibling"));
    let mut turns: Vec<_> = calls
        .iter()
        .map(|tool| {
            let full = tool.arguments.to_string();
            // Leave only the closing brace missing. If the runner repairs the
            // JSON and dispatches it, tool_invocations can match the original arguments.
            let prefix = full
                .strip_suffix('}')
                .expect("the scripted tool takes object arguments");
            let parts = std::iter::once(tool)
                .chain(sibling.iter())
                .cloned()
                .map(ScriptedPart::ToolCall)
                .collect();
            ScriptedTurn::new(parts)
                .with_tool_argument_prefix(&tool.tool_call_id, prefix)
                .with_finish_reason(raw_length)
        })
        .collect();
    let (answer, expected_text) = case.mint.text("after-cut");
    turns
        .push(ScriptedTurn::new(vec![answer]).with_finish_reason(finish_reason::raw_stop(source)?));
    let mut session = case.open("length-cut", Script { turns }).await?;
    let params = case.params(
        &session,
        "make the scripted calls",
        &case.mint.next("length-cut"),
        None,
    );
    let end = crate::run_parameters::send(case, &mut session, params, None).await?;
    let result = run_result(&session.route, &end.run_id).await?;
    if three {
        if result.run_state() != RunState::Error
            || result.error.as_ref().is_none_or(|error| {
                error.provider_code != provider_codes::OUTPUT_LIMIT || error.class != "permanent"
            })
        {
            return Err(format!(
                "expected permanent output_limit on the third cut step, got {result:?}"
            ));
        }
    } else if result.run_state() != RunState::Completed
        || result
            .final_message
            .as_ref()
            .is_none_or(|message| message.text != expected_text)
    {
        return Err(format!(
            "expected a completed run with the answer after the cut call, got {result:?}"
        ));
    }
    let requests = case
        .subject
        .model_requests(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let expected_requests = if three { 3 } else { 2 };
    if requests.len() != expected_requests {
        return Err(format!(
            "expected {expected_requests} model requests, got {}",
            requests.len()
        ));
    }
    let listed = crate::tool_calls::list(&session.route, &end.run_id, 10).await?;
    for (index, tool) in calls.iter().enumerate() {
        let record = listed
            .iter()
            .find(|record| record.tool_call_id == tool.tool_call_id)
            .ok_or_else(|| {
                format!(
                    "run.tool_calls did not list the cut call {}",
                    tool.tool_call_id
                )
            })?;
        if case.subject.tool_invocations(&tool.arguments).await != 0
            || record.outcome != outcomes::NOT_EXECUTED
            || record.dispatched_to.is_some()
            || record.result_bytes.is_none()
        {
            return Err(format!(
                "cut call must get a not-executed result without dispatch: {record:?}"
            ));
        }
        if let Some(next) = requests.get(index + 1) {
            let received = next
                .tool_results
                .iter()
                .find(|result| result.tool_call_id == tool.tool_call_id);
            if received.is_none_or(|result| {
                !result.is_error
                    || result.text.trim().is_empty()
                    || result.code.as_deref() != Some(tool_result_reasons::ARGUMENTS_INCOMPLETE)
            }) {
                return Err(format!(
                    "next model request needs an arguments_incomplete error for {}: {received:?}",
                    tool.tool_call_id
                ));
            }
        }
    }
    if let Some(sibling) = sibling {
        if case.subject.tool_invocations(&sibling.arguments).await != 1 {
            return Err(
                "the completed sibling call of the cut step did not run exactly once".into(),
            );
        }
    }
    Ok(Ending::Passed)
}

// Short standalone words avoid relying on one tokenizer. Eight words per
// requested token allows for estimator and delta-boundary differences, but
// the full stream is eight times even that margin.
const CAP: u64 = 16;
const WORD: &str = "hello ";
const SCRIPT_WORDS: usize = 1_024;
const MARGIN_BYTES: usize = CAP as usize * 8 * WORD.len();

pub(crate) async fn runner_cap<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let Some(source) = case.subject.model_finish_reason_source() else {
        return Ok(Ending::Inapplicable(
            "adapter cannot script raw model finish reasons".into(),
        ));
    };
    let text = WORD.repeat(SCRIPT_WORDS);
    let mut session = case
        .open(
            "runner-cap",
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(text.clone())])
                    .with_finish_reason(finish_reason::raw_stop(source)?)],
            },
        )
        .await?;
    if !case
        .subject
        .configure_runner_enforced_cap(&session.name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "adapter has no scripted provider path without an output cap".into(),
        ));
    }
    require_declared(&session.route, groups::MAX_OUTPUT_TOKENS, ops::SESSION_SEND).await?;
    require_declared(&session.route, groups::FINISH_REASON, ops::RUN_RESULT).await?;
    let mut params = case.params(
        &session,
        "answer at length",
        &case.mint.next("runner-cap"),
        None,
    );
    params["generation"]["max_output_tokens"] = json!(CAP);
    let end = crate::run_parameters::send(case, &mut session, params, None).await?;
    let requests = case
        .subject
        .model_requests(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    if !matches!(requests.as_slice(), [request] if request.max_output_tokens.is_none()) {
        return Err(format!(
            "expected one model request with no output cap, got {requests:?}"
        ));
    }
    let result = run_result(&session.route, &end.run_id).await?;
    if result.run_state() != RunState::Completed {
        return Err(format!(
            "a truncated text reply must complete the run: {result:?}"
        ));
    }
    let output = &result
        .final_message
        .ok_or("truncated run has no final message")?
        .text;
    if output.is_empty() || output.len() > MARGIN_BYTES || !text.starts_with(output) {
        return Err(format!("expected a nonempty prefix of at most {MARGIN_BYTES} bytes for cap {CAP}, got {} bytes", output.len()));
    }
    finish_reason::check(case, &session.route, &end.run_id, Some(Finish::Length)).await?;
    Ok(Ending::Passed)
}
