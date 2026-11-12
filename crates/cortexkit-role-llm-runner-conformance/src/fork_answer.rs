//! Independent model-ingress checks for private fork conversations.

use std::{collections::BTreeSet, path::Path};

use cortexkit_role_harness::CrashDriver;
use cortexkit_role_llm_runner::{
    errors,
    fork_answer::{ForkAskReply, ForkPrepareReply},
    ops,
    run::RunError,
    session_change::RefreshRequest,
};
use futures_util::future::{select, Either};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all,
        wait_run_end, Mint,
    },
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn, ScriptedUsage},
    RecordingRoutingPeer, RunnerRoute,
};

type Check<T = ()> = Result<T, String>;
mod raw;

fn text(value: &str) -> ScriptedTurn {
    ScriptedTurn::new(vec![ScriptedPart::Text(value.into())])
        .with_finish_reason("end_turn")
        .with_usage(ScriptedUsage::new(17, 3, 0, 11))
}

async fn prepare<R: RunnerRoute>(route: &R) -> Check<String> {
    let reply: ForkPrepareReply = decode(
        "fork_prepare",
        expect_ok(
            "fork_prepare",
            call(route, ops::SESSION_FORK_PREPARE, json!({})).await?,
        )?,
    )?;
    if reply.fork_id.is_empty() {
        return Err("fork_prepare returned an empty fork_id".into());
    }
    Ok(reply.fork_id)
}

fn params(fork: &str, id: &str, question: &str) -> Value {
    json!({"fork_id":fork,"ask_id":id,"question":question})
}

async fn ask<R: RunnerRoute>(route: &R, fork: &str, id: &str, question: &str) -> Check<Value> {
    let value = expect_ok(
        "fork_ask",
        call(route, ops::SESSION_FORK_ASK, params(fork, id, question)).await?,
    )?;
    let _: ForkAskReply = decode("fork_ask", value.clone())?;
    Ok(value)
}

async fn refused<R: RunnerRoute>(
    route: &R,
    fork: &str,
    id: &str,
    question: &str,
    code: &str,
) -> Check {
    expect_code(
        "fork_ask",
        &expect_error(
            "fork_ask",
            call(route, ops::SESSION_FORK_ASK, params(fork, id, question)).await?,
        )?,
        code,
    )
}

async fn end<R: RunnerRoute>(route: &R, fork: &str) -> Check {
    let value = expect_ok(
        "fork_end",
        call(route, ops::SESSION_FORK_END, json!({"fork_id":fork})).await?,
    )?;
    if value != json!({"fork_id":fork,"ended":true}) {
        return Err(format!("wrong fork_end answer: {value}"));
    }
    Ok(())
}

async fn replay<R: RunnerRoute>(
    route: &R,
    fork: &str,
    id: &str,
    question: &str,
    first: &Value,
) -> Check {
    if &ask(route, fork, id, question).await? != first {
        return Err("replay changed answer, finish or usage".into());
    }
    Ok(())
}

async fn seed<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    label: &str,
    turns: Vec<ScriptedTurn>,
    peer: Option<RecordingRoutingPeer>,
) -> Check<Session<S::Route>> {
    let mut all = vec![text("head answer")];
    all.extend(turns);
    let session = case.open(label, Script { turns: all }).await?;
    if let Some(peer) = peer {
        case.subject
            .install_routing_peer(&session.name, peer)
            .await
            .map_err(|e| e.to_string())?;
    }
    start(case, session, None).await
}

async fn start<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    mut session: Session<S::Route>,
    tier: Option<&str>,
) -> Check<Session<S::Route>> {
    let owner = case.subject.owner_stamp();
    let members =
        json!({"fork_callers":[crate::send_helpers::stamp_module("fork_answer", &owner)?]});
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, members.as_object().unwrap())
        .ok_or("ForkAnswer requires send_fields_with_plan for fork_callers")?;
    let id = case.mint.next("fork-head-send");
    let mut request = case.params(&session, "head prompt", &id, None);
    request.as_object_mut().unwrap().extend(fields);
    if let Some(tier) = tier {
        request["service_tier"] = json!(tier);
    }
    let receipt = decode(
        "head send",
        expect_ok(
            "head send",
            call(&session.route, ops::SESSION_SEND, request).await?,
        )?,
    )?;
    session.first = Some((id, "head prompt".into()));
    wait_run_end(case.subject, &session.route, case.declared, None, &receipt).await?;
    Ok(session)
}

async fn requests<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    session: &str,
) -> Check<Vec<crate::RawModelRequestObservation>> {
    case.subject
        .raw_model_requests(session)
        .await
        .map_err(|e| e.to_string())
}

fn contains_value(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.contains(needle),
        Value::Array(a) => a.iter().any(|v| contains_value(v, needle)),
        Value::Object(o) => o.values().any(|v| contains_value(v, needle)),
        _ => false,
    }
}

fn contains(bytes: &[u8], needle: &str) -> bool {
    serde_json::from_slice::<Value>(bytes).is_ok_and(|value| contains_value(&value, needle))
}

async fn base_request<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    session: &str,
) -> Check<Vec<u8>> {
    requests(case, session)
        .await?
        .into_iter()
        .rev()
        .find(|r| r.fork_id.is_none())
        .map(|r| r.body)
        .ok_or("no raw head model request was captured".into())
}

async fn fork_request<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    session: &str,
    fork: &str,
) -> Check<Vec<u8>> {
    requests(case, session)
        .await?
        .into_iter()
        .rev()
        .find(|r| r.fork_id.as_deref() == Some(fork))
        .map(|r| r.body)
        .ok_or("no raw request attributed to this fork_id".into())
}

async fn clock<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    session: &str,
    ms: u64,
) -> Check {
    case.subject
        .advance_fork_clock(session, ms)
        .await
        .map_err(|e| e.to_string())
}

async fn refresh<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    callers: Value,
    generation: u64,
) -> Check<crate::Reply> {
    let mut plan = case
        .subject
        .send_fields_with_plan(&session.name, true, &Map::new())
        .and_then(|mut f| f.remove("plan"))
        .and_then(|p| p.as_object().cloned())
        .ok_or("ForkAnswer requires a plan fixture for refresh")?;
    plan.insert("fork_callers".into(), callers);
    call(
        &session.route,
        ops::SESSION_REFRESH,
        serde_json::to_value(RefreshRequest::new(plan, "immediately", generation)).unwrap(),
    )
    .await
}

pub(crate) async fn run<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    name: &str,
) -> Check<Ending> {
    let describe = case.checked_describe().await?;
    match name {
        "fork_answer_base_matches_digest" | "fork_answer_refuses_mismatched_base" => {
            wire_rules(&describe)?;
            let mut session = seed(
                case,
                name,
                vec![
                    text("later head"),
                    text("fork answer"),
                    text("next fork answer"),
                ],
                None,
            )
            .await?;
            let base = base_request(case, &session.name).await?;
            let fork = prepare(&session.route).await?;
            for (op, request) in [
                (ops::SESSION_FORK_PREPARE, json!({"unexpected":true})),
                (
                    ops::SESSION_FORK_ASK,
                    json!({"fork_id":fork,"ask_id":"unknown-member","question":"question","unexpected":true}),
                ),
                (
                    ops::SESSION_FORK_END,
                    json!({"fork_id":fork,"unexpected":true}),
                ),
            ] {
                let error = expect_error(
                    "strict fork request",
                    call(&session.route, op, request).await?,
                )?;
                expect_field(
                    "strict fork request",
                    &error,
                    errors::INVALID_PARAMS,
                    "unexpected",
                )?;
            }
            let other = prepare(&session.route).await?;
            if other == fork {
                return Err("fork ids were reused".into());
            }
            if name == "fork_answer_refuses_mismatched_base" {
                case.run_prompt(&mut session, "advance head after prepare")
                    .await?;
            }
            ask(&session.route, &fork, "first", "private question").await?;
            let request = fork_request(case, &session.name, &fork).await?;
            raw::extends_base(&base, &request).map_err(|error| {
                format!(
                    "head base SHA-256 {}: {error}",
                    cortexkit_role_llm_runner::content_digest::sha256_hex(&base)
                )
            })?;
            if !contains(&request, "private question") {
                return Err("fork request did not append its question".into());
            }
            if name == "fork_answer_refuses_mismatched_base" {
                let messages = read_all(&session.route).await?.1;
                let mid = &messages.first().ok_or("no removable base message")?.mid;
                if let Err(reason) = case
                    .subject
                    .revert_session(case.handle.inner(), &session.name, mid)
                    .await
                {
                    return Ok(Ending::Inapplicable(format!(
                        "fork_answer_refuses_mismatched_base requires revert_session: {reason}"
                    )));
                }
                let count = requests(case, &session.name).await?.len();
                refused(
                    &session.route,
                    &fork,
                    "after-revert",
                    "must not reach model",
                    errors::FORK_BASE_UNAVAILABLE,
                )
                .await?;
                if requests(case, &session.name).await?.len() != count {
                    return Err("base refusal reached model".into());
                }
            } else {
                ask(&session.route, &fork, "second", "second question").await?;
                let next = fork_request(case, &session.name, &fork).await?;
                if !contains(&next, "private question") || !contains(&next, "second question") {
                    return Err("fork lost its private conversation".into());
                }
            }
        }
        "fork_answer_refuses_tool_call" => {
            let tool = case.mint.call("forbidden-fork-tool");
            let session = seed(
                case,
                name,
                vec![ScriptedTurn::new(vec![
                    ScriptedPart::Text("before tool".into()),
                    ScriptedPart::ToolCall(tool.clone()),
                    ScriptedPart::Text("after tool".into()),
                ])
                .with_usage(ScriptedUsage::new(17, 3, 0, 11))],
                None,
            )
            .await?;
            let fork = prepare(&session.route).await?;
            let first = ask(&session.route, &fork, "tool", "try a tool").await?;
            raw::extends_base(
                &base_request(case, &session.name).await?,
                &fork_request(case, &session.name, &fork).await?,
            )?;
            if first["answer"] != "before tool"
                || first["finish"] != "tool_call_refused"
                || case.subject.tool_invocations(&tool.arguments).await != 0
            {
                return Err("fork tool call was not refused at the per-call gate".into());
            }
            let models = requests(case, &session.name).await?.len();
            replay(&session.route, &fork, "tool", "try a tool", &first).await?;
            end(&session.route, &fork).await?;
            replay(&session.route, &fork, "tool", "try a tool", &first).await?;
            if requests(case, &session.name).await?.len() != models {
                return Err("tool-refused replay spent tokens".into());
            }
            refused(
                &session.route,
                &fork,
                "new",
                "new question",
                errors::FORK_ENDED,
            )
            .await?;
        }
        "fork_answer_model_failure_finishes" => {
            let failure = RunError::new("transient", "scripted model failure", "provider_error");
            let session = seed(
                case,
                name,
                vec![
                    text("partial failed text").with_model_failure(failure),
                    text("recovered"),
                ],
                None,
            )
            .await?;
            let fork = prepare(&session.route).await?;
            let first = ask(&session.route, &fork, "failure", "failed private question").await?;
            if first["finish"] != "model_error" || first["answer"] != "" {
                return Err("model failure was not a stored empty model_error answer".into());
            }
            replay(
                &session.route,
                &fork,
                "failure",
                "failed private question",
                &first,
            )
            .await?;
            ask(&session.route, &fork, "next", "fresh question").await?;
            let request = fork_request(case, &session.name, &fork).await?;
            if contains(&request, "failed private question")
                || contains(&request, "partial failed text")
            {
                return Err("failed question or partial text joined the branch".into());
            }
        }
        "fork_answer_idle_expiry" => {
            idle(case, describe.fork_idle_s.ok_or("missing fork_idle_s")?).await?
        }
        "fork_answer_never_writes_head" => isolation(case).await?,
        "fork_answer_admits_only_fork_callers" => callers(case).await?,
        "fork_answer_account_for_pins_head_credential"
        | "fork_answer_conversation_key_bounded"
        | "fork_answer_account_held_refusal"
        | "fork_answer_account_blocked_refusal" => return routing(case, name).await,
        other => return Err(format!("unknown fork case {other}")),
    }
    Ok(Ending::Passed)
}

async fn idle<S: LlmRunnerSubject<Route: RunnerRoute>>(case: &Case<'_, S>, seconds: u64) -> Check {
    let ttl = seconds
        .checked_mul(1000)
        .ok_or("fork_idle_s overflows milliseconds")?;
    let session = seed(
        case,
        "fork-idle",
        vec![
            text("first"),
            text("held answer"),
            text("last"),
            text("ended held answer"),
        ],
        None,
    )
    .await?;
    let unused = prepare(&session.route).await?;
    clock(case, &session.name, ttl).await?;
    refused(
        &session.route,
        &unused,
        "expired",
        "question",
        errors::UNKNOWN_FORK,
    )
    .await?;
    let fork = prepare(&session.route).await?;
    clock(case, &session.name, ttl - 1).await?;
    let first = ask(&session.route, &fork, "one", "first question").await?;
    clock(case, &session.name, ttl - 1).await?;
    replay(&session.route, &fork, "one", "first question", &first).await?;
    clock(case, &session.name, 1).await?;
    refused(
        &session.route,
        &fork,
        "two",
        "expired question",
        errors::UNKNOWN_FORK,
    )
    .await?;
    replay(&session.route, &fork, "one", "first question", &first).await?;
    let fork = prepare(&session.route).await?;
    case.subject
        .hold_next_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let ask_future = Box::pin(ask(&session.route, &fork, "held", "held question"));
    let wait = Box::pin(case.subject.await_model_step(&session.name));
    let pending = match select(ask_future, wait).await {
        Either::Left(_) => return Err("held ask completed before release".into()),
        Either::Right((result, pending)) => {
            result.map_err(|e| e.to_string())?;
            pending
        }
    };
    let checks = async {
        refused(
            &session.route,
            &fork,
            "held",
            "held question",
            errors::FORK_BUSY,
        )
        .await?;
        refused(
            &session.route,
            &fork,
            "other",
            "another question",
            errors::FORK_BUSY,
        )
        .await?;
        clock(case, &session.name, ttl + 1).await
    }
    .await;
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let held = pending.await?;
    checks?;
    if held["answer"] != "held answer" {
        return Err("idle expiry ended an in-flight ask".into());
    }
    clock(case, &session.name, ttl - 1).await?;
    ask(
        &session.route,
        &fork,
        "after",
        "still alive after held completion",
    )
    .await?;
    let fork = prepare(&session.route).await?;
    case.subject
        .hold_next_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let pending = match select(
        Box::pin(ask(
            &session.route,
            &fork,
            "end-held",
            "question while ending",
        )),
        Box::pin(case.subject.await_model_step(&session.name)),
    )
    .await
    {
        Either::Left(_) => return Err("ask finished before fork_end could run".into()),
        Either::Right((ready, pending)) => {
            ready.map_err(|e| e.to_string())?;
            pending
        }
    };
    let ended = async {
        end(&session.route, &fork).await?;
        end(&session.route, &fork).await
    }
    .await;
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let answer = pending.await?;
    ended?;
    if answer["answer"] != "ended held answer" {
        return Err("fork_end cancelled an in-flight answer".into());
    }
    replay(
        &session.route,
        &fork,
        "end-held",
        "question while ending",
        &answer,
    )
    .await?;
    refused(
        &session.route,
        &fork,
        "after-end",
        "new question",
        errors::FORK_ENDED,
    )
    .await?;
    Ok(())
}

async fn isolation<S: LlmRunnerSubject<Route: RunnerRoute>>(case: &Case<'_, S>) -> Check {
    let mut session = seed(
        case,
        "fork-isolation",
        vec![text("head continues"), text("private answer")],
        None,
    )
    .await?;
    let fork = prepare(&session.route).await?;
    case.subject
        .hold_next_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let receipt =
        crate::send_helpers::send(case, &mut session, None, "hold head", &Map::new()).await?;
    case.subject
        .await_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let before = read_all(&session.route).await?.1;
    let before_head = head(&session.route).await?;
    let subscription =
        serde_json::to_value(cortexkit_role_llm_runner::subscribe::SubscribeRequest::new(
            cortexkit_role_llm_runner::subscribe::SubscribeFrom::Start,
        ))
        .unwrap();
    let events = session
        .route
        .subscribe(subscription.clone())
        .await
        .map_err(|e| e.message)?;
    let live = serde_json::to_value(cortexkit_role_llm_runner::subscribe::SubscribeRequest::new(
        cortexkit_role_llm_runner::subscribe::SubscribeFrom::Live,
    ))
    .unwrap();
    let quiet = session
        .route
        .subscribe(live.clone())
        .await
        .map_err(|e| e.message)?;
    let checked = async {
        let (observed, answer) = futures_util::future::join(
            session.route.subscribe(live),
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                ask(&session.route, &fork, "private", "private prompt"),
            ),
        )
        .await;
        if observed.map_err(|e| e.message)? != quiet {
            return Err("fork emitted live head subscription events".into());
        }
        let answer = answer.map_err(|_| "fork waited on the head's held model step")??;
        if answer["answer"] != "private answer" {
            return Err("fork did not use its scripted answer".into());
        }
        if read_all(&session.route).await?.1 != before || head(&session.route).await? != before_head
        {
            return Err("fork changed the head transcript, run or usage".into());
        }
        if session
            .route
            .subscribe(subscription)
            .await
            .map_err(|e| e.message)?
            != events
        {
            return Err("fork emitted an event on the head subscription".into());
        }
        Ok(())
    }
    .await;
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    wait_run_end(case.subject, &session.route, case.declared, None, &receipt).await?;
    checked
}

async fn callers<S: LlmRunnerSubject<Route: RunnerRoute>>(case: &Case<'_, S>) -> Check {
    for malformed in [
        json!(null),
        json!("not a list"),
        json!({}),
        json!(["caller", 1]),
        json!([""]),
        json!(["duplicate", "duplicate"]),
    ] {
        let invalid = case.open("invalid-fork-plan", Script::default()).await?;
        let members = json!({"fork_callers":malformed});
        let fields = case
            .subject
            .send_fields_with_plan(&invalid.name, true, members.as_object().unwrap())
            .ok_or("ForkAnswer requires send_fields_with_plan")?;
        let mut request = case.params(
            &invalid,
            "must not be committed",
            &case.mint.next("invalid-plan"),
            None,
        );
        request.as_object_mut().unwrap().extend(fields);
        let before = call(&invalid.route, ops::SESSION_HEAD, json!({})).await?;
        let error = expect_error(
            "malformed admitted fork_callers",
            call(&invalid.route, ops::SESSION_SEND, request).await?,
        )?;
        expect_field(
            "malformed admitted fork_callers",
            &error,
            errors::INVALID_PARAMS,
            "plan.fork_callers",
        )?;
        if call(&invalid.route, ops::SESSION_HEAD, json!({})).await? != before
            || !requests(case, &invalid.name).await?.is_empty()
        {
            return Err("invalid admission committed transcript or reached model".into());
        }
    }
    let session = seed(
        case,
        "fork-callers",
        vec![text("stored"), text("secret held answer")],
        None,
    )
    .await?;
    let fork = prepare(&session.route).await?;
    let foreign = case
        .subject
        .session_route(
            case.handle.inner(),
            &session.name,
            &case.subject.stranger_stamp(),
        )
        .await
        .map_err(|e| e.to_string())?;
    expect_code(
        "unlisted prepare",
        &expect_error(
            "unlisted prepare",
            call(&foreign, ops::SESSION_FORK_PREPARE, json!({})).await?,
        )?,
        errors::SCOPE_OWNER_MISMATCH,
    )?;
    refused(&foreign, &fork, "one", "question", errors::UNKNOWN_FORK).await?;
    expect_code(
        "foreign end",
        &expect_error(
            "foreign end",
            call(&foreign, ops::SESSION_FORK_END, json!({"fork_id":fork})).await?,
        )?,
        errors::UNKNOWN_FORK,
    )?;
    let first = ask(&session.route, &fork, "one", "question").await?;
    for malformed in [
        json!(null),
        json!("not a list"),
        json!({}),
        json!(["caller", 1]),
        json!([""]),
        json!(["duplicate", "duplicate"]),
    ] {
        let before = read_all(&session.route).await?.1;
        let error = expect_error(
            "malformed fork_callers",
            refresh(case, &session, malformed, 1).await?,
        )?;
        expect_field(
            "malformed fork_callers",
            &error,
            errors::INVALID_PARAMS,
            "plan.fork_callers",
        )?;
        if read_all(&session.route).await?.1 != before {
            return Err("malformed refresh wrote history".into());
        }
        replay(&session.route, &fork, "one", "question", &first).await?;
    }
    case.subject
        .hold_next_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let pending = match select(
        Box::pin(call(
            &session.route,
            ops::SESSION_FORK_ASK,
            params(&fork, "held", "held question"),
        )),
        Box::pin(case.subject.await_model_step(&session.name)),
    )
    .await
    {
        Either::Left(_) => return Err("held ask completed before caller refresh".into()),
        Either::Right((ready, pending)) => {
            ready.map_err(|e| e.to_string())?;
            pending
        }
    };
    let checked = async {
        expect_ok(
            "remove caller",
            refresh(case, &session, json!([]), 1).await?,
        )?;
        expect_code(
            "owner without fork_callers",
            &expect_error(
                "owner without fork_callers",
                call(&session.route, ops::SESSION_FORK_PREPARE, json!({})).await?,
            )?,
            errors::SCOPE_OWNER_MISMATCH,
        )?;
        refused(
            &session.route,
            &fork,
            "one",
            "question",
            errors::UNKNOWN_FORK,
        )
        .await?;
        refused(
            &session.route,
            &fork,
            "new",
            "new question",
            errors::UNKNOWN_FORK,
        )
        .await
    }
    .await;
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let reply = pending.await?;
    checked?;
    expect_code(
        "revoked held ask",
        &expect_error("revoked held ask", reply)?,
        errors::UNKNOWN_FORK,
    )?;
    expect_ok(
        "restore caller",
        refresh(
            case,
            &session,
            json!([crate::send_helpers::stamp_module(
                "restore caller",
                &case.subject.owner_stamp()
            )?]),
            2,
        )
        .await?,
    )?;
    for (id, q) in [
        ("one", "question"),
        ("held", "held question"),
        ("new", "new question"),
    ] {
        refused(&session.route, &fork, id, q, errors::UNKNOWN_FORK).await?;
    }
    let next = prepare(&session.route).await?;
    if next == fork {
        return Err("re-adding caller reused a revoked fork".into());
    }
    Ok(())
}

fn key(value: &Value) -> String {
    format!(
        "fork:{}",
        cortexkit_role_llm_runner::content_digest::sha256_hex(&serde_jcs::to_vec(value).unwrap())
    )
}

fn released(peer: &RecordingRoutingPeer, admission: &Value) -> Check {
    let count = peer
        .calls()
        .iter()
        .filter(|c| {
            c.method == "route.account_observe"
                && c.body["kind"] == "run_finished"
                && c.body["admission_id"] == *admission
        })
        .count();
    if count != 1 {
        return Err(format!(
            "admission {admission} released {count} times, expected once"
        ));
    }
    Ok(())
}

async fn routing<S: LlmRunnerSubject<Route: RunnerRoute>>(
    case: &Case<'_, S>,
    name: &str,
) -> Check<Ending> {
    let mapping = matches!(
        name,
        "fork_answer_account_held_refusal" | "fork_answer_account_blocked_refusal"
    );
    if mapping && !case.subject.routing_peer_error_replies() {
        return Ok(Ending::Inapplicable(format!(
            "{name}: routing_peer_error_replies is unsupported"
        )));
    }
    let describe = case.checked_describe().await?;
    let Some(method) = describe.routed_accounts.as_ref().and_then(|m| m.first()) else {
        return Ok(Ending::Inapplicable(format!(
            "{name} needs routed_accounts"
        )));
    };
    let fixture = case
        .subject
        .routed_account_fixture(method)
        .await
        .map_err(|e| e.to_string())?;
    let peer = RecordingRoutingPeer::new(method, fixture.clone()).map_err(|e| e.to_string())?;
    let session = case
        .open(
            name,
            Script {
                turns: vec![text("head answer"), text("fork answer")],
            },
        )
        .await?;
    case.subject
        .install_routing_peer(&session.name, peer.clone())
        .await
        .map_err(|e| e.to_string())?;
    let bounded = name == "fork_answer_conversation_key_bounded";
    let head_key = "h".repeat(256);
    if bounded
        && !case
            .subject
            .configure_fork_head_key(&session.name, &head_key)
            .await
            .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(format!(
            "{name}: configure_fork_head_key is unsupported"
        )));
    }
    let fast = !bounded
        && !mapping
        && describe.declares(cortexkit_role_llm_runner::capabilities::SERVICE_TIER)
        && case
            .subject
            .configure_service_tier(&session.name, name)
            .await
            .map_err(|e| e.to_string())?;
    let session = start(case, session, fast.then_some("fast")).await?;
    let head_calls = peer.calls();
    let head_call = head_calls
        .iter()
        .find(|c| c.method == "route.account_for")
        .ok_or("head made no routing admission")?;
    if bounded && head_call.body["conversation_key"] != head_key {
        return Err(
            "configure_fork_head_key did not produce the stated 256-character head key".into(),
        );
    }
    let frozen_head = head(&session.route).await?;
    let fork = prepare(&session.route).await?;
    let initial_models = requests(case, &session.name).await?.len();
    let mut attempts = 0;
    if mapping {
        let held = name == "fork_answer_account_held_refusal";
        let codes = if held {
            &[
                "account_hold",
                "account_pool_exhausted",
                "account_selection_unavailable",
            ][..]
        } else {
            &[
                "no_account",
                "account_conflict",
                "admission_conflict",
                "pin_expired",
            ][..]
        };
        for code in codes {
            peer.set_account_refusal(Some((*code).into()));
            let error = expect_error(
                "routing refusal",
                call(
                    &session.route,
                    ops::SESSION_FORK_ASK,
                    params(&fork, "retry", "private question"),
                )
                .await?,
            )?;
            expect_code(
                "routing refusal",
                &error,
                if held {
                    errors::FORK_ACCOUNT_HELD
                } else {
                    errors::FORK_ACCOUNT_BLOCKED
                },
            )?;
            if error.detail.as_ref().and_then(|d| d.get("routing_code")) != Some(&json!(code)) {
                return Err(format!("routing refusal lost its routing code: {error:?}"));
            }
            attempts += 1;
            if requests(case, &session.name).await?.len() != initial_models
                || head(&session.route).await? != frozen_head
            {
                return Err("routing refusal spent tokens or changed the head".into());
            }
        }
        peer.set_account_refusal(None);
    } else if !bounded {
        let head_tier = head_call.body["service_tier"]
            .as_str()
            .unwrap_or("standard");
        let mismatch_tier = if head_tier == "standard" {
            "fast"
        } else {
            "standard"
        };
        for (credential, tier, mismatch) in [
            ("other-healthy-credential", head_tier, "credential"),
            (
                fixture["credential_id"]
                    .as_str()
                    .ok_or("credential_id missing")?,
                mismatch_tier,
                "service_tier",
            ),
        ] {
            peer.set_account_override(Some((credential.into(), tier.into())));
            let error = expect_error(
                "pinned account",
                call(
                    &session.route,
                    ops::SESSION_FORK_ASK,
                    params(&fork, "retry", "private question"),
                )
                .await?,
            )?;
            expect_code("pinned account", &error, errors::FORK_ACCOUNT_HELD)?;
            if error.detail.as_ref().and_then(|d| d.get("mismatch")) != Some(&json!(mismatch)) {
                return Err(format!("wrong account mismatch detail: {error:?}"));
            }
            attempts += 1;
            if requests(case, &session.name).await?.len() != initial_models
                || head(&session.route).await? != frozen_head
            {
                return Err("refused fork spent tokens or changed the head".into());
            }
            let calls = peer.calls();
            let admitted = calls
                .iter()
                .rev()
                .find(|c| c.method == "route.account_for")
                .ok_or("no fork admission")?;
            released(&peer, &admitted.body["admission_id"])?;
        }
        peer.set_account_override(None);
    }
    let answer = ask(&session.route, &fork, "retry", "private question").await?;
    if answer["answer"] != "fork answer" {
        return Err("fork retry did not reach the model".into());
    }
    for (field, expected) in [
        ("input_tokens", 17),
        ("output_tokens", 11),
        ("cached_input_tokens", 3),
    ] {
        if answer["usage"][field].as_u64() != Some(expected) {
            return Err(format!("fork spend not returned in ask usage: {answer}"));
        }
    }
    if head(&session.route).await? != frozen_head {
        return Err("fork spend joined head usage".into());
    }
    let calls = peer.calls();
    let admissions: Vec<_> = calls
        .iter()
        .skip(head_calls.len())
        .filter(|c| c.method == "route.account_for")
        .collect();
    if admissions.len() != attempts + 1 {
        return Err("retry did not get exactly one fresh admission per attempt".into());
    }
    let expected_key = key(&json!({"fork_id":fork,"head_key":head_call.body["conversation_key"]}));
    for (i, admit) in admissions.iter().enumerate() {
        let body = &admit.body;
        let expected_id = key(&json!({"ask_id":"retry","attempt":i+1,"fork_id":fork}));
        if body["conversation_key"] != expected_key
            || expected_key.len() != 69
            || body["admission_id"] != expected_id
        {
            return Err(format!("fork routing key derivation differs: {body}"));
        }
        for field in [
            "provider",
            "model",
            "affinity_model",
            "pool",
            "service_tier",
        ] {
            if body.get(field) != head_call.body.get(field) {
                return Err(format!("fork admission changed frozen {field}"));
            }
        }
        if body["hold_wait_max_s"] != 0
            || body.get("decision_id").is_some_and(|v| !v.is_null())
            || body
                .get("parent_conversation_key")
                .is_some_and(|v| !v.is_null())
            || body.pointer("/requested_account/account_id") != fixture.get("account_id")
            || body.pointer("/requested_account/credential_id") != fixture.get("credential_id")
        {
            return Err(format!(
                "fork admission did not pin the head's routing selection: {body}"
            ));
        }
        if !mapping || i == attempts {
            released(&peer, &body["admission_id"])?;
        }
    }
    if !mapping && !bounded && !fast {
        return Ok(Ending::PassedWithInapplicableRule("fork_answer.account_pin stepped-down tier requires service_tier and configure_service_tier".into()));
    }
    Ok(Ending::Passed)
}

/// Restart-specific cases use a separate root so no other case's live handle is
/// stopped, and inspect the same persisted forks through the recovered route.
pub(crate) async fn restart<S: LlmRunnerSubject<Route: RunnerRoute>>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
    name: &str,
) -> Check<Ending> {
    let root = work_dir.join(name);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let handle = driver.spawn(&root).await.map_err(|e| e.to_string())?;
    let case = Case {
        subject,
        handle: &handle,
        declared,
        mint,
    };
    let describe = case.checked_describe().await?;
    let peer = if let Some(method) = describe
        .routed_accounts
        .as_ref()
        .and_then(|methods| methods.first())
    {
        Some(
            RecordingRoutingPeer::new(
                method,
                subject
                    .routed_account_fixture(method)
                    .await
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let tool = mint.call("replay-tool-call");
    let turns = if name == "fork_answer_ask_id_replays" {
        vec![
            text("stored answer"),
            ScriptedTurn::new(vec![
                ScriptedPart::Text("tool prefix".into()),
                ScriptedPart::ToolCall(tool),
            ])
            .with_usage(ScriptedUsage::new(17, 3, 0, 11)),
        ]
    } else {
        vec![
            text("stored answer"),
            text("interrupted answer"),
            text("after recovery"),
        ]
    };
    let session = seed(&case, name, turns, peer.clone()).await?;
    let fork = prepare(&session.route).await?;
    let first = ask(&session.route, &fork, "stored", "stored question").await?;
    refused(
        &session.route,
        &fork,
        "stored",
        "different question",
        errors::ASK_ID_REUSE,
    )
    .await?;
    for (id, q, field) in [
        ("".to_owned(), "question", "ask_id"),
        ("界".repeat(257), "question", "ask_id"),
        ("empty-question".into(), "", "question"),
    ] {
        let error = expect_error(
            "ask bounds",
            call(&session.route, ops::SESSION_FORK_ASK, params(&fork, &id, q)).await?,
        )?;
        expect_field("ask bounds", &error, errors::INVALID_PARAMS, field)?;
    }
    let count = requests(&case, &session.name).await?.len();
    replay(&session.route, &fork, "stored", "stored question", &first).await?;
    if requests(&case, &session.name).await?.len() != count {
        return Err("lost-reply replay made another model call".into());
    }
    let mut interrupted = None;
    let mut tool_replay = None;
    if name == "fork_answer_survives_restart" {
        subject
            .hold_next_model_step(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        let pending = match select(
            Box::pin(call(
                &session.route,
                ops::SESSION_FORK_ASK,
                params(&fork, "inflight", "interrupted question"),
            )),
            Box::pin(subject.await_model_step(&session.name)),
        )
        .await
        {
            Either::Left(_) => return Err("ask was not in flight at restart".into()),
            Either::Right((result, pending)) => {
                result.map_err(|e| e.to_string())?;
                pending
            }
        };
        drop(pending);
        interrupted = Some("inflight");
    } else {
        let tool_fork = prepare(&session.route).await?;
        let answer = ask(&session.route, &tool_fork, "tool", "tool question").await?;
        if answer["finish"] != "tool_call_refused" {
            return Err("scripted tool call did not terminate the replay fork".into());
        }
        replay(&session.route, &tool_fork, "tool", "tool question", &answer).await?;
        tool_replay = Some((tool_fork, answer));
        end(&session.route, &fork).await?;
        end(&session.route, &fork).await?;
        // Deletion exactly at the minimum retention cutoff is permitted.
        clock(&case, &session.name, 3_599_999).await?;
        replay(&session.route, &fork, "stored", "stored question", &first).await?;
    }
    let session_name = session.name.clone();
    let model_count = requests(&case, &session_name).await?.len();
    let admission = peer
        .as_ref()
        .and_then(|p| {
            p.calls()
                .into_iter()
                .rev()
                .find(|c| c.method == "route.account_for")
        })
        .map(|c| c.body["admission_id"].clone());
    drop(handle);
    let handle = subject.restart(&root).await.map_err(|e| e.to_string())?;
    let route = subject
        .session_route(&handle, &session_name, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    replay(&route, &fork, "stored", "stored question", &first).await?;
    if let Some((fork, answer)) = &tool_replay {
        replay(&route, fork, "tool", "tool question", answer).await?;
    }
    if subject
        .raw_model_requests(&session_name)
        .await
        .map_err(|e| e.to_string())?
        .len()
        != model_count
    {
        return Err("restart replay made another model call".into());
    }
    if let (Some(peer), Some(admission)) = (&peer, &admission) {
        released(peer, admission)?;
    }
    if let Some(id) = interrupted {
        let recovered = ask(&route, &fork, id, "interrupted question").await?;
        if recovered["finish"] != "model_error" || recovered["answer"] != "" {
            return Err("restart failed to store model_error for an in-flight ask".into());
        }
        replay(&route, &fork, id, "interrupted question", &recovered).await?;
        ask(&route, &fork, "after", "after restart question").await?;
        let body = subject
            .raw_model_requests(&session_name)
            .await
            .map_err(|e| e.to_string())?
            .into_iter()
            .rev()
            .find(|r| r.fork_id.as_deref() == Some(&fork))
            .ok_or("no recovered fork request")?
            .body;
        if contains(&body, "interrupted question")
            || !contains(&body, "stored question")
            || !contains(&body, "stored answer")
        {
            return Err("restart lost the branch or kept the failed question".into());
        }
    } else {
        let messages = read_all(&route).await?.1;
        if let Err(reason) = subject
            .revert_session(
                &handle,
                &session_name,
                &messages.first().ok_or("no base message")?.mid,
            )
            .await
        {
            return Ok(Ending::Inapplicable(format!("fork_answer_ask_id_replays requires revert_session for replay after base loss: {reason}")));
        }
        replay(&route, &fork, "stored", "stored question", &first).await?;
        if let Some((fork, answer)) = &tool_replay {
            replay(&route, fork, "tool", "tool question", answer).await?;
        }
    }
    if peer.is_none() && interrupted.is_some() {
        return Ok(Ending::PassedWithInapplicableRule(
            "fork_answer.restart admission release requires routed_accounts".into(),
        ));
    }
    Ok(Ending::Passed)
}

fn wire_rules(describe: &cortexkit_role_llm_runner::describe::RoleDescribe) -> Check {
    use cortexkit_role_llm_runner::describe::{check_describe, DescribeProblem};
    let raw = serde_json::to_value(describe).unwrap();
    for (change, code) in [
        (None, "fork_idle_s_missing"),
        (Some(json!(0)), "fork_idle_s_invalid"),
        (Some(json!(null)), "fork_idle_s_invalid"),
    ] {
        let mut changed = raw.clone();
        if let Some(value) = change {
            changed["fork_idle_s"] = value;
        } else {
            changed.as_object_mut().unwrap().remove("fork_idle_s");
        }
        if !check_describe(&changed)
            .err()
            .is_some_and(|errors| errors.contains(&DescribeProblem::Undecodable(code.into())))
        {
            return Err(format!("describe checker did not name {code}"));
        }
    }
    let mut changed = raw;
    changed["capabilities"]
        .as_array_mut()
        .unwrap()
        .retain(|v| v != "fork_answer");
    if !check_describe(&changed).err().is_some_and(|errors| {
        errors.contains(&DescribeProblem::Undecodable(
            "fork_idle_s_undeclared".into(),
        ))
    }) {
        return Err("describe checker accepted an undeclared fork fact".into());
    }
    for code in [
        "unknown_fork",
        "fork_ended",
        "fork_busy",
        "ask_id_reuse",
        "fork_base_unavailable",
        "fork_unsupported",
        "fork_account_held",
        "fork_account_blocked",
    ] {
        if !errors::CODES.contains(&code)
            || errors::is_retryable(code) != matches!(code, "fork_busy" | "fork_account_held")
            || errors::is_account_wait(code)
        {
            return Err(format!("fork code/outcome advice differs for {code}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
