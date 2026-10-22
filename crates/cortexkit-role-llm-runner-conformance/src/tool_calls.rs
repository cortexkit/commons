//! `run_tool_calls` cases, run only when the subject declares
//! `run_tool_calls`: a run's tool calls read as a list, without decoding any
//! message body.

use cortexkit_role_llm_runner::{
    capabilities as groups, errors, ops,
    read::ReadMessage,
    tool_calls::{outcomes, RunToolCall, ToolCallsPage, ToolCallsRequest, LIMIT_FIELD},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::Case,
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all},
    route::{Reply, RunnerRoute},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    tags::send_and_wait,
};

type Check = Result<(), String>;

/// How many pages a full listing follows before it gives up.
const MAX_PAGES: usize = 1000;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "run_tool_calls_listed_in_dispatch_order" => dispatch_order(case).await,
        "run_tool_calls_unknown_run_refused" => unknown_run(case).await,
        "run_tool_calls_indeterminate_until_result" => indeterminate(case, false).await,
        "run_tool_calls_positions_identify_siblings" => positions(case).await,
        "run_tool_calls_result_ordinal_tracks_storage" => indeterminate(case, true).await,
        other => Err(format!("unknown run_tool_calls case {other}")),
    }
}

async fn raw<R: RunnerRoute>(route: &R, params: Value) -> Result<Reply, String> {
    call(route, ops::RUN_TOOL_CALLS, params).await
}

async fn page<R: RunnerRoute>(
    route: &R,
    request: &ToolCallsRequest,
) -> Result<ToolCallsPage, String> {
    let body = expect_ok(
        "run.tool_calls",
        raw(route, serde_json::to_value(request).expect("serializes")).await?,
    )?;
    decode("run.tool_calls", body)
}

/// Every call of run `run_id`: pages of at most `limit` calls, each request
/// passing the previous page's `next` back as `after`, until a page has no
/// `next`.
pub(crate) async fn list<R: RunnerRoute>(
    route: &R,
    run_id: &str,
    limit: u64,
) -> Result<Vec<RunToolCall>, String> {
    let mut calls = Vec::new();
    let mut after: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut request = ToolCallsRequest::new(run_id).with_limit(limit);
        if let Some(after) = &after {
            request = request.with_after(after);
        }
        let page = page(route, &request).await?;
        if page.run_id != run_id {
            return Err(format!(
                "run.tool_calls for {run_id} answered a page of {}",
                page.run_id
            ));
        }
        if page.calls.len() as u64 > limit {
            return Err(format!(
                "run.tool_calls with limit {limit} answered {} calls",
                page.calls.len()
            ));
        }
        let empty = page.calls.is_empty();
        calls.extend(page.calls);
        match page.next {
            None => return Ok(calls),
            Some(_) if empty => {
                return Err("run.tool_calls answered no call but a next cursor".into())
            }
            next => after = next,
        }
    }
    Err(format!("the list did not end within {MAX_PAGES} pages"))
}

/// The session's last run, which the case's own send started.
async fn last_run<R: RunnerRoute>(route: &R) -> Result<String, String> {
    head(route)
        .await?
        .last_run_state
        .map(|last| last.run_id)
        .ok_or_else(|| "session.head names no run after the send".into())
}

/// The message at `ordinal` holds the call's arguments.
fn carried(messages: &[ReadMessage], call: &RunToolCall, arguments: &Value) -> Check {
    let marker = arguments["marker"].as_str().unwrap_or_default();
    match messages.iter().find(|m| m.ordinal == call.ordinal) {
        Some(message) if message.message.to_string().contains(marker) => Ok(()),
        _ => Err(format!(
            "run.tool_calls names ordinal {} as carrying the call with arguments {arguments}, \
             but that message does not hold them",
            call.ordinal
        )),
    }
}

async fn dispatch_order<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    // Two sibling calls in the first step and a third in the second. Siblings
    // get distinct model ids, as a real provider never repeats an id within one
    // response and a host may key tool parts by it. The ids sort against
    // dispatch order, and the third reuses the first's id (allowed across
    // steps, §5), so a runner that orders or identifies calls by model id
    // fails; only position and key tell the two "listed-b" calls apart.
    let scripted: Vec<_> = ["listed-b", "listed-a", "listed-b"]
        .into_iter()
        .map(|id| case.mint.call(id))
        .collect();
    let (last, _) = case.mint.text("listed-final");
    let mut session = case
        .open(
            "run-tool-calls",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![
                        ScriptedPart::ToolCall(scripted[0].clone()),
                        ScriptedPart::ToolCall(scripted[1].clone()),
                    ]),
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(scripted[2].clone())]),
                    ScriptedTurn::new(vec![last]),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::RUN_TOOL_CALLS, ops::RUN_TOOL_CALLS)
        .await?;
    let prompt = case.mint.next("listed-prompt");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let run_id = last_run(&session.route).await?;
    let calls = list(&session.route, &run_id, 1).await?;
    let (_, messages) = read_all(&session.route).await?;
    if calls.len() != scripted.len() {
        return Err(format!(
            "run.tool_calls listed {} calls, the run made {}",
            calls.len(),
            scripted.len()
        ));
    }
    let module = case.subject.tool_provider_module();
    let mut keys = std::collections::BTreeSet::new();
    for (index, (listed, scripted)) in calls.iter().zip(&scripted).enumerate() {
        if listed.args != scripted.arguments {
            return Err(format!(
                "call {index} of run.tool_calls has args {}, but the run's call {index} in \
                 dispatch order had {}",
                listed.args, scripted.arguments
            ));
        }
        if listed.name != scripted.tool || listed.tool_call_id != scripted.tool_call_id {
            return Err(format!(
                "call {index} is named {} with id {}, not {} with id {}",
                listed.name, listed.tool_call_id, scripted.tool, scripted.tool_call_id
            ));
        }
        if listed.outcome != outcomes::SUCCEEDED
            || listed.dispatched_to.as_deref() != Some(module.as_str())
            || listed.result_bytes.is_none_or(|bytes| bytes == 0)
        {
            return Err(format!(
                "call {index} of a completed run reads {listed:?}: expected outcome succeeded, \
                 dispatched to {module}, with the size of its stored result"
            ));
        }
        if let Some(key) = &listed.call_key {
            if !keys.insert(key.clone()) {
                return Err(format!("two listed calls share call_key {key}"));
            }
        }
        carried(&messages, listed, &scripted.arguments)?;
    }
    Ok(())
}

async fn unknown_run<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, _) = case.mint.text("unknown-run-answer");
    let mut session = case
        .open(
            "run-tool-calls-unknown",
            Script {
                turns: vec![ScriptedTurn::new(vec![answer])],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::RUN_TOOL_CALLS, ops::RUN_TOOL_CALLS)
        .await?;
    let prompt = case.mint.next("unknown-run-prompt");
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    let what = "run.tool_calls for a run the session does not have";
    let error = expect_error(
        what,
        raw(
            &session.route,
            json!({"run_id": case.mint.next("no-such-run")}),
        )
        .await?,
    )?;
    expect_code(what, &error, errors::UNKNOWN_RUN)?;
    let run_id = last_run(&session.route).await?;
    let empty = list(&session.route, &run_id, 10).await?;
    if !empty.is_empty() {
        return Err(format!("a run without tool calls listed {empty:?}"));
    }
    for (what, params, field) in [
        (
            "run.tool_calls with a misspelled cursor",
            json!({"run_id": run_id, "from": 1}),
            "from",
        ),
        (
            "run.tool_calls with limit 0",
            json!({"run_id": run_id, "limit": 0}),
            LIMIT_FIELD,
        ),
    ] {
        let error = expect_error(what, raw(&session.route, params).await?)?;
        expect_field(what, &error, errors::INVALID_PARAMS, field)?;
    }
    Ok(())
}

async fn indeterminate<S>(case: &Case<'_, S>, locations: bool) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut held = case.mint.call("listed-held");
    held.hold = true;
    let (last, _) = case.mint.text("held-final");
    let session = case
        .open(
            "run-tool-calls-held",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
                    ScriptedTurn::new(vec![last]),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::RUN_TOOL_CALLS, ops::RUN_TOOL_CALLS)
        .await?;
    let prompt = case.mint.next("held-prompt");
    let params = crate::tags::params(
        case,
        &session,
        None,
        &prompt,
        &case.mint.next("send"),
        &Map::new(),
    );
    expect_ok(
        "session.send",
        call(&session.route, ops::SESSION_SEND, params).await?,
    )?;
    case.subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("waiting for the held call: {e}"))?;
    let run_id = last_run(&session.route).await?;
    let during = list(&session.route, &run_id, 10).await?;
    match during.as_slice() {
        [call] if call.outcome == outcomes::INDETERMINATE && call.result_bytes.is_none() => {}
        other => {
            return Err(format!(
                "while its call is held, run.tool_calls lists {other:?}: expected one \
                 indeterminate call with no stored result"
            ))
        }
    }
    if locations {
        let call = &during[0];
        if call.result_ordinal.is_some() || call.position != Some(0) {
            return Err(format!(
                "open call must have position 0 and no result_ordinal: {call:?}"
            ));
        }
        let (_, messages) = read_all(&session.route).await?;
        // A message embedding both the call and its result is not finished
        // until the tool returns, so its ordinal need not be readable yet.
        if messages
            .iter()
            .any(|message| message.ordinal == call.ordinal)
        {
            carried(&messages, call, &held.arguments)?;
        }
    }
    case.subject
        .release_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("releasing the held call: {e}"))?;
    for _ in 0..crate::drive::MAX_POLLS {
        let after = list(&session.route, &run_id, 10).await?;
        if let [call] = after.as_slice() {
            if call.outcome == outcomes::SUCCEEDED && call.result_bytes.is_some() {
                if locations {
                    let ordinal = call
                        .result_ordinal
                        .filter(|ordinal| *ordinal >= call.ordinal)
                        .ok_or_else(|| {
                            format!("finished call needs result_ordinal >= ordinal: {call:?}")
                        })?;
                    let (_, messages) = read_all(&session.route).await?;
                    carried(&messages, call, &held.arguments)?;
                    if !messages.iter().any(|message| {
                        message.ordinal == ordinal
                            && message
                                .message
                                .to_string()
                                .contains(&crate::drive::result_marker(&held))
                    }) {
                        return Err(format!(
                            "result_ordinal {ordinal} does not locate the stored result"
                        ));
                    }
                }
                return Ok(());
            }
        }
        case.subject.pause().await;
    }
    Err("after its release, the held call never listed as succeeded with a stored result".into())
}

async fn positions<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let first = case.mint.call("position-first");
    let second = case.mint.call("position-second");
    let mut session = case
        .open(
            "call-positions",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![
                        ScriptedPart::Reasoning("not a call".into()),
                        ScriptedPart::ToolCall(first.clone()),
                        ScriptedPart::Text("between calls".into()),
                        ScriptedPart::ToolCall(second.clone()),
                    ]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("done".into())]),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::RUN_TOOL_CALLS, ops::RUN_TOOL_CALLS)
        .await?;
    send_and_wait(case, &mut session, None, "call position probe", &Map::new()).await?;
    let calls = list(&session.route, &last_run(&session.route).await?, 10).await?;
    if calls.len() != 2 {
        return Err(format!("expected two calls, got {calls:?}"));
    }
    let (_, messages) = read_all(&session.route).await?;
    let mut ordinal = None;
    for (position, scripted) in [first, second].iter().enumerate() {
        let listed = calls
            .iter()
            .find(|call| call.args == scripted.arguments)
            .ok_or("missing scripted call")?;
        if listed.position != Some(position as u32)
            || ordinal.is_some_and(|ordinal| ordinal != listed.ordinal)
        {
            return Err(format!(
                "sibling positions must be 0 and 1 at the same requesting ordinal: {calls:?}"
            ));
        }
        ordinal = Some(listed.ordinal);
        carried(&messages, listed, &scripted.arguments)?;
    }
    Ok(())
}
