//! Queue promotion is observed through the real route and the send's ordinary receipt.

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all,
        wait_run_end, Mint, MAX_POLLS,
    },
    route::RunnerRoute,
    send_helpers::{params, require_declared, send, send_and_wait},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn},
};
use cortexkit_role_harness::CrashDriver;
use cortexkit_role_llm_runner::{
    capabilities as groups,
    describe::RoleDescribe,
    errors, ops,
    promote::{PromoteReason, PromoteReply, PromoteRequest},
    send::{SendReply, SendState},
};
use serde_json::{json, Map, Value};
use std::{collections::BTreeSet, path::Path};

pub(crate) const REPLAY: &str = "send_promote_request_replay";

async fn promote<R: RunnerRoute>(
    route: &R,
    send_id: &str,
    request_id: &str,
) -> Result<PromoteReply, String> {
    decode(
        "session.promote",
        expect_ok(
            "session.promote",
            call(
                route,
                ops::SESSION_PROMOTE,
                serde_json::to_value(PromoteRequest::new(send_id, request_id)).unwrap(),
            )
            .await?,
        )?,
    )
}

async fn receipt<R: RunnerRoute>(route: &R, request: &Value) -> Result<SendReply, String> {
    decode(
        "send receipt",
        expect_ok(
            "send receipt",
            call(route, ops::SESSION_SEND, request.clone()).await?,
        )?,
    )
}

fn expected(reply: PromoteReply, wanted: PromoteReply) -> Result<(), String> {
    if reply == wanted {
        Ok(())
    } else {
        Err(format!("expected {wanted:?}, got {reply:?}"))
    }
}

fn delivered(reply: &SendReply, kind: &str, run: Option<&str>) -> Result<(), String> {
    if !reply
        .delivered
        .as_ref()
        .is_some_and(|d| d.r#as == kind && d.r#ref.is_some())
        || run.is_some_and(|run| reply.run_id.as_deref() != Some(run))
    {
        return Err(format!(
            "expected a {kind} receipt for run {run:?}, got {reply:?}"
        ));
    }
    if kind == "turn" && reply.delivered.as_ref().unwrap().r#ref != reply.run_id {
        return Err("a turn receipt must name its own run".into());
    }
    Ok(())
}

async fn settled<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    request: &Value,
    kind: &str,
) -> Result<SendReply, String>
where
    S::Route: RunnerRoute,
{
    for _ in 0..MAX_POLLS {
        let reply = receipt(route, request).await?;
        if reply.delivered.as_ref().is_some_and(|d| d.r#as == kind) {
            delivered(&reply, kind, None)?;
            if case.declared.contains(&Capability::SendStatus) {
                let status: SendReply = decode(
                    "session.send_status",
                    expect_ok(
                        "session.send_status",
                        call(
                            route,
                            ops::SESSION_SEND_STATUS,
                            json!({"send_id":request["send_id"]}),
                        )
                        .await?,
                    )?,
                )?;
                if (&status.delivered, &status.run_id) != (&reply.delivered, &reply.run_id) {
                    return Err(
                        "promotion changed send_status or the send's ordinary receipt".into(),
                    );
                }
            }
            return Ok(reply);
        }
        case.subject.pause().await;
    }
    Err(format!("queued send never reached its {kind} receipt"))
}

async fn active<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
    plan: Option<&Map<String, Value>>,
) -> Result<(Session<S::Route>, ScriptedToolCall, String), String>
where
    S::Route: RunnerRoute,
{
    let mut held = case.mint.call("promote-held");
    held.hold = true;
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
                    ScriptedTurn::new(vec![case.mint.text("after-boundary").0]),
                    ScriptedTurn::new(vec![case.mint.text("after-next-turn").0]),
                ],
            },
        )
        .await?;
    require_declared(&session.route, groups::SEND_PROMOTE, ops::SESSION_PROMOTE).await?;
    let fields = plan
        .map(|members| {
            case.subject
                .send_fields_with_plan(&session.name, true, members)
                .ok_or("adapter cannot build a plan with a named sender")
        })
        .transpose()?;
    let first = send(case, &mut session, fields.as_ref(), "opening", &Map::new()).await?;
    case.subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| e.to_string())?;
    let run_id = first
        .run_id
        .or(head(&session.route).await?.last_run_state.map(|r| r.run_id))
        .ok_or("held run has no run id")?;
    Ok((session, held, run_id))
}

async fn queue<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    route: &S::Route,
    marker: &str,
) -> Result<Value, String>
where
    S::Route: RunnerRoute,
{
    let request = params(
        case,
        session,
        None,
        marker,
        &case.mint.next("queued"),
        &json!({"delivery":"queue"}).as_object().unwrap().clone(),
    );
    let reply = receipt(route, &request).await?;
    if reply.send_state() != SendState::Pending || reply.submission_id.is_none() {
        return Err(format!(
            "send must really be queued before promotion: {reply:?}"
        ));
    }
    Ok(request)
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    if name == "send_promote_unsupported" || name == "send_promote_requires_steer" {
        let session = case.open(name, Script::default()).await?;
        let describe: RoleDescribe = decode(
            "role.describe",
            expect_ok(
                "role.describe",
                call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
            )?,
        )?;
        let group = describe.declares(groups::SEND_PROMOTE);
        let steer = describe.declares(groups::STEER);
        if name == "send_promote_requires_steer" {
            if group && !steer {
                return Err("send_promote_without_steer".into());
            }
            return Ok(if group {
                Ending::Passed
            } else {
                Ending::Inapplicable("send_promote not declared".into())
            });
        }
        if group && steer {
            return Ok(Ending::Inapplicable(
                "send_promote and steer are declared".into(),
            ));
        }
        let reply = call(
            &session.route,
            ops::SESSION_PROMOTE,
            json!({"send_id":"missing", "request_id":"unsupported"}),
        )
        .await?;
        expect_code(
            "undeclared promotion",
            &expect_error("undeclared promotion", reply)?,
            errors::UNSUPPORTED,
        )?;
        return Ok(Ending::Passed);
    }
    if name == "send_promote_parked" {
        return parked(case, false).await.map(|_| Ending::Passed);
    }
    if name == "send_promote_sender_namespace" {
        return namespace(case).await;
    }
    let (session, held, joined) = active(case, name, None).await?;
    let marker = case.mint.next("queued-words");
    let request = queue(case, &session, &session.route, &marker).await?;
    let send_id = request["send_id"].as_str().unwrap();
    let request_id = case.mint.next("promote-request");
    let result: Result<(), String> = async {
        if name == "send_promote_steer_admission" {
            let stranger = case
                .subject
                .session_route(
                    case.handle.inner(),
                    &session.name,
                    &case.subject.stranger_stamp(),
                )
                .await
                .map_err(|e| e.to_string())?;
            let reply = call(
                &stranger,
                ops::SESSION_PROMOTE,
                json!({"send_id":send_id, "request_id":request_id}),
            )
            .await?;
            expect_code(
                "promotion without steer grant",
                &expect_error("promotion without steer grant", reply)?,
                errors::SCOPE_OWNER_MISMATCH,
            )?;
            let unchanged = receipt(&session.route, &request).await?;
            if unchanged.send_state() != SendState::Pending {
                return Err("refused promotion changed the queue".into());
            }
            return Ok(());
        }
        if name == "send_promote_turn_ended_first" {
            case.subject
                .release_tool_call(&held.arguments)
                .await
                .map_err(|e| e.to_string())?;
            let delivered = settled(case, &session.route, &request, "turn").await?;
            wait_run_end(
                case.subject,
                &session.route,
                case.declared,
                None,
                &delivered,
            )
            .await
            .map_err(|reason| format!("queued send's run did not end: {reason}"))?;
            let before = receipt(&session.route, &request).await?;
            let messages = read_all(&session.route).await?.1;
            let models = case
                .subject
                .model_invocations(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            expected(
                promote(&session.route, send_id, &request_id).await?,
                PromoteReply::Delivered,
            )?;
            if receipt(&session.route, &request).await? != before
                || read_all(&session.route).await?.1 != messages
                || case
                    .subject
                    .model_invocations(&session.name)
                    .await
                    .map_err(|e| e.to_string())?
                    != models
            {
                return Err("delivered promotion ran or changed the send again".into());
            }
            return Ok(());
        }
        expected(
            promote(&session.route, send_id, &request_id).await?,
            PromoteReply::Promoted,
        )?;
        let models_before = if name == "send_promote_active_next_step" {
            case.subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?
                .len()
        } else {
            0
        };
        if name == "send_promote_run_ends_before_step" {
            expect_ok(
                "end run before next step",
                call(
                    &session.route,
                    ops::RUN_CANCEL,
                    json!({"run_id":joined, "at":"now"}),
                )
                .await?,
            )?;
        }
        case.subject
            .release_tool_call(&held.arguments)
            .await
            .map_err(|e| e.to_string())?;
        let kind = if name == "send_promote_run_ends_before_step" {
            "turn"
        } else {
            "step"
        };
        let after = settled(case, &session.route, &request, kind).await?;
        wait_run_end(case.subject, &session.route, case.declared, None, &after).await?;
        delivered(&after, kind, (kind == "step").then_some(joined.as_str()))?;
        if kind == "turn" && after.run_id.as_deref() == Some(joined.as_str()) {
            return Err(
                "promoted send must start the next turn after its original run ends".into(),
            );
        }
        if name == "send_promote_active_next_step" {
            let requests = case
                .subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            if !requests
                .get(models_before)
                .is_some_and(|request| request.rendered.iter().any(|text| text.contains(&marker)))
            {
                return Err("promoted words did not reach the run's next model step".into());
            }
            expected(
                promote(&session.route, send_id, &request_id).await?,
                PromoteReply::Promoted,
            )?;
        }
        let messages = read_all(&session.route).await?.1;
        if messages
            .iter()
            .filter(|m| m.message.to_string().contains(&marker))
            .count()
            != 1
        {
            return Err("promoted words must be stored exactly once".into());
        }
        Ok(())
    }
    .await;
    // Always release the provider hold, even when admission or an assertion failed.
    if name == "send_promote_steer_admission" || result.is_err() {
        let _ = case.subject.release_tool_call(&held.arguments).await;
    }
    result?;
    Ok(Ending::Passed)
}

struct Replay {
    session: String,
    send_id: String,
    request_id: String,
    answer: PromoteReply,
}

async fn reuse<R: RunnerRoute>(route: &R, request_id: &str) -> Result<(), String> {
    let reply = call(
        route,
        ops::SESSION_PROMOTE,
        json!({"send_id":"different-send", "request_id":request_id}),
    )
    .await?;
    let error = expect_error("promotion request-id reuse", reply)?;
    expect_field(
        "promotion request-id reuse",
        &error,
        errors::REQUEST_ID_REUSE,
        "send_id",
    )
}

async fn parked<S: LlmRunnerSubject>(case: &Case<'_, S>, replay: bool) -> Result<Replay, String>
where
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            if replay {
                REPLAY
            } else {
                "send_promote_parked"
            },
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("opening").0]),
                    ScriptedTurn::new(vec![case.mint.text("queued-turn").0]),
                ],
            },
        )
        .await?;
    require_declared(&session.route, groups::SEND_PROMOTE, ops::SESSION_PROMOTE).await?;
    send_and_wait(case, &mut session, None, "opening", &Map::new()).await?;
    if !case
        .subject
        .hold_retention_queue(&session.name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Err(
            "send_promote requires an adapter fixture that parks a queue without an active run"
                .into(),
        );
    }
    let request = queue(
        case,
        &session,
        &session.route,
        &case.mint.next("parked-words"),
    )
    .await?;
    let send_id = request["send_id"].as_str().unwrap().to_owned();
    let request_id = case.mint.next("promote-request");
    let models = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let before = receipt(&session.route, &request).await?;
    let first = promote(&session.route, &send_id, &request_id).await;
    let check: Result<PromoteReply, String> = async {
        let answer = first?;
        expected(
            answer.clone(),
            PromoteReply::NotPromoted {
                reason: PromoteReason::NoActiveRun,
            },
        )?;
        if receipt(&session.route, &request).await? != before
            || case
                .subject
                .model_invocations(&session.name)
                .await
                .map_err(|e| e.to_string())?
                != models
        {
            return Err("no_active_run changed the queued send or started a run".into());
        }
        if replay {
            expected(
                promote(&session.route, &send_id, &request_id).await?,
                answer.clone(),
            )?;
            reuse(&session.route, &request_id).await?;
        }
        Ok(answer)
    }
    .await;
    let released = case
        .subject
        .release_retention_queue(&session.name)
        .await
        .map_err(|e| e.to_string());
    let answer = check?;
    released?;
    let after = settled(case, &session.route, &request, "turn").await?;
    wait_run_end(case.subject, &session.route, case.declared, None, &after).await?;
    if replay {
        expected(
            promote(&session.route, &send_id, &request_id).await?,
            answer.clone(),
        )?;
        expected(
            promote(&session.route, &send_id, &case.mint.next("new-request")).await?,
            PromoteReply::Delivered,
        )?;
        reuse(&session.route, &request_id).await?;
    }
    Ok(Replay {
        session: session.name,
        send_id,
        request_id,
        answer,
    })
}

async fn namespace<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let Some(sender) = case.subject.plan_sender() else {
        return Err("send_promote sender isolation requires a plan_sender fixture".into());
    };
    let members = json!({"senders":[{"module":sender.module, "idle_start":true}]})
        .as_object()
        .unwrap()
        .clone();
    let (session, held, _) = active(case, "send_promote_sender_namespace", Some(&members)).await?;
    let check: Result<(), String> = async {
        let route = case
            .subject
            .session_route(case.handle.inner(), &session.name, &sender.stamp)
            .await
            .map_err(|e| e.to_string())?;
        let request = queue(case, &session, &session.route, "owner-only-words").await?;
        let send_id = request["send_id"].as_str().unwrap();
        for id in ["missing-send", send_id] {
            expected(
                promote(&route, id, &case.mint.next("promote-request")).await?,
                PromoteReply::NotQueued,
            )?;
        }
        if receipt(&session.route, &request).await?.send_state() != SendState::Pending {
            return Err("another sender's promotion changed the queue".into());
        }
        let mut own = request.clone();
        own["prompt"] = json!("named-sender-words");
        if receipt(&route, &own).await?.send_state() != SendState::Pending {
            return Err("equal send ids from different senders must not collide".into());
        }
        expected(
            promote(&route, send_id, &case.mint.next("own-request")).await?,
            PromoteReply::Promoted,
        )?;
        if case.declared.contains(&Capability::Retract) {
            let withdrawn = queue(case, &session, &session.route, "withdrawn-words").await?;
            let pending = receipt(&session.route, &withdrawn).await?;
            expect_ok(
                "withdraw queued send",
                call(
                    &session.route,
                    ops::SESSION_RETRACT,
                    json!({"submission_id":pending.submission_id.unwrap()}),
                )
                .await?,
            )?;
            expected(
                promote(
                    &session.route,
                    withdrawn["send_id"].as_str().unwrap(),
                    &case.mint.next("withdrawn-request"),
                )
                .await?,
                PromoteReply::NotQueued,
            )?;
        }
        Ok(())
    }
    .await;
    let released = case
        .subject
        .release_tool_call(&held.arguments)
        .await
        .map_err(|e| e.to_string());
    check?;
    released?;
    Ok(Ending::Passed)
}

/// A dedicated runner root lets the kit check replay across a real restart
/// without restarting the shared runner that other cases use.
pub(crate) async fn restart<S: LlmRunnerSubject>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let root = work_dir.join("send-promote-replay");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let handle = driver.spawn(&root).await.map_err(|e| e.to_string())?;
    let replay = parked(
        &Case {
            subject,
            handle: &handle,
            declared,
            mint,
        },
        true,
    )
    .await?;
    drop(handle);
    let handle = subject.restart(&root).await.map_err(|e| e.to_string())?;
    let route = subject
        .session_route(&handle, &replay.session, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    expected(
        promote(&route, &replay.send_id, &replay.request_id).await?,
        replay.answer,
    )?;
    reuse(&route, &replay.request_id).await
}

#[cfg(test)]
mod tests;
