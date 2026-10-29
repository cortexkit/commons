//! Cancellation at a model-step boundary, gated by the live discovery answer.

use cortexkit_role_llm_runner::{
    describe::check_describe,
    errors, ops,
    run::{cancel_at, states, RunCancelReply, RunCancelRequest},
    send::SendReply,
};
use serde_json::json;

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, holding, read_all,
        result_marker, run_result, wait_run_end,
    },
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let discovery = case.open("cancel-discovery", Script::default()).await?;
    let raw = expect_ok(
        "role.describe",
        call(&discovery.route, ops::ROLE_DESCRIBE, json!({})).await?,
    )?;
    let describe = check_describe(&raw).map_err(|errors| format!("role.describe: {errors:?}"))?;
    let points = describe
        .run_cancel
        .as_ref()
        .map(|cancel| cancel.at.as_slice())
        .unwrap_or(&[]);
    if !points.iter().any(|point| point == cancel_at::STEP_BOUNDARY) {
        return Ok(Ending::Skipped {
            missing: vec![Capability::RunCancel],
            reason: "role.describe.run_cancel.at does not list step_boundary".into(),
        });
    }
    match name {
        "run_cancel_step_boundary_finishes_tool_batch" => boundary(case).await?,
        "run_cancel_unlisted_at_refused" => unlisted(case, points).await?,
        "run_cancel_step_boundary_escalates_to_now" => escalation(case).await?,
        _ => return Err(format!("unknown cancellation case {name}")),
    }
    Ok(Ending::Passed)
}

async fn cancel<R: RunnerRoute>(route: &R, run_id: &str, at: &str) -> Result<(), String> {
    let body = expect_ok(
        "run.cancel",
        call(
            route,
            ops::RUN_CANCEL,
            serde_json::to_value(RunCancelRequest::new(run_id).with_at(at)).unwrap(),
        )
        .await?,
    )?;
    let reply: RunCancelReply = decode("run.cancel", body)?;
    if reply != RunCancelReply::Ack {
        return Err(format!("run.cancel at {at} answered {reply:?}, not ack"));
    }
    Ok(())
}

async fn start_held<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
) -> Result<(SendReply, String), String>
where
    S::Route: RunnerRoute,
{
    start_held_after(case, session, 0).await
}

async fn start_held_after<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    before: usize,
) -> Result<(SendReply, String), String>
where
    S::Route: RunnerRoute,
{
    case.subject
        .hold_next_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let send = case.mint.next("cancel-send");
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(
                &session.route,
                ops::SESSION_SEND,
                case.params(session, "cancel probe", &send, None),
            )
            .await?,
        )?,
    )?;
    case.subject
        .await_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let run_id = match &reply.run_id {
        Some(run_id) => run_id.clone(),
        None => {
            head(&session.route)
                .await?
                .last_run_state
                .ok_or("the held send has no observable run")?
                .run_id
        }
    };
    if run_result(&session.route, &run_id)
        .await?
        .run_state()
        .is_terminal()
    {
        return Err("the model hold did not keep the run non-terminal".into());
    }
    let count = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    if count != before + 1 {
        return Err(format!(
            "the held model received {} new requests, not one",
            count.saturating_sub(before)
        ));
    }
    Ok((reply, run_id))
}

pub(crate) async fn by_interrupt<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    use cortexkit_role_llm_runner::capabilities;
    if !case
        .checked_describe()
        .await?
        .declares(capabilities::RUN_CANCEL_BY_INTERRUPT)
    {
        return Ok(Ending::Inapplicable(
            "run_cancel_by_interrupt is not declared".into(),
        ));
    }
    let missing: Vec<_> = [
        Capability::RunCancel,
        Capability::DeliveryOrder,
        Capability::Queue,
        Capability::RunOps,
        Capability::TranscriptReads,
    ]
    .into_iter()
    .filter(|cap| !case.declared.contains(cap))
    .collect();
    if !missing.is_empty() {
        return Ok(Ending::Skipped {
            missing,
            reason: "the cancellation probe needs held runs and reads".into(),
        });
    }
    let grant = name != "run_cancel_by_interrupt_without_grant_refused";
    let (session, sender) = crate::delivery_order::sender_session(
        case,
        "interrupt-cancel",
        vec![ScriptedTurn::new(vec![ScriptedPart::Text("held".into())])],
        false,
        grant,
    )
    .await?;
    let (target, route) = if name == "run_cancel_by_interrupt_other_session_refused" {
        let other = case
            .open(
                "interrupt-cancel-other",
                Script {
                    turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                        "other held".into(),
                    )])],
                },
            )
            .await?;
        let identity = case.subject.plan_sender().ok_or("no plan sender")?;
        let route = case
            .subject
            .session_route(case.handle.inner(), &other.name, &identity.stamp)
            .await
            .map_err(|e| e.to_string())?;
        (other, route)
    } else {
        (session, sender)
    };
    let opening_requests = usize::from(name != "run_cancel_by_interrupt_other_session_refused");
    let (reply, run_id) = start_held_after(case, &target, opening_requests).await?;
    let at = if name == "run_cancel_by_interrupt_non_now_refused" {
        cancel_at::STEP_BOUNDARY
    } else {
        cancel_at::NOW
    };
    let response = call(&route, ops::RUN_CANCEL, json!({"run_id":run_id, "at":at})).await?;
    let checked = async {
        if name == "run_cancel_by_interrupt_now_cancelled" {
            let answer: RunCancelReply =
                decode("sender cancel", expect_ok("sender cancel", response)?)?;
            if answer != RunCancelReply::Ack {
                return Err("sender cancel did not acknowledge".into());
            }
            // The model stays held: reaching cancelled must not depend on its answer.
            wait_run_end(case.subject, &target.route, case.declared, None, &reply).await?;
            if run_result(&target.route, &run_id).await?.state != states::CANCELLED {
                return Err("sender cancel did not end cancelled".into());
            }
        } else {
            let error = expect_error("ungranted sender cancel", response)?;
            expect_code(
                "ungranted sender cancel",
                &error,
                errors::SCOPE_OWNER_MISMATCH,
            )?;
            if run_result(&target.route, &run_id)
                .await?
                .run_state()
                .is_terminal()
            {
                return Err("refused sender cancel ended the run".into());
            }
            cancel(&target.route, &run_id, cancel_at::NOW).await?;
        }
        Ok::<(), String>(())
    }
    .await;
    case.subject
        .release_model_step(&target.name)
        .await
        .map_err(|e| e.to_string())?;
    checked?;
    Ok(Ending::Passed)
}

async fn boundary<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let mut first = case.mint.call("boundary-first");
    first.hold = true;
    let second = case.mint.call("boundary-second");
    let forbidden = case.mint.call("boundary-forbidden-next-step");
    let session = case
        .open(
            "cancel-boundary",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![
                        ScriptedPart::ToolCall(first.clone()),
                        ScriptedPart::ToolCall(second.clone()),
                    ]),
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(forbidden.clone())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("unexpected next step".into())]),
                ],
            },
        )
        .await?;
    let (reply, run_id) = start_held(case, &session).await?;
    let stranger = case
        .subject
        .session_route(
            case.handle.inner(),
            &session.name,
            &case.subject.stranger_stamp(),
        )
        .await
        .map_err(|e| e.to_string())?;
    let error = expect_error(
        "non-owner boundary cancel",
        call(
            &stranger,
            ops::RUN_CANCEL,
            json!({"run_id":run_id, "at":cancel_at::STEP_BOUNDARY}),
        )
        .await?,
    )?;
    expect_code(
        "non-owner boundary cancel",
        &error,
        errors::SCOPE_OWNER_MISMATCH,
    )?;
    cancel(&session.route, &run_id, cancel_at::STEP_BOUNDARY).await?;
    cancel(&session.route, &run_id, cancel_at::STEP_BOUNDARY).await?;
    if run_result(&session.route, &run_id)
        .await?
        .run_state()
        .is_terminal()
    {
        return Err("boundary cancellation ended the run before its model step finished".into());
    }
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    case.subject
        .await_tool_call(&first.arguments)
        .await
        .map_err(|e| e.to_string())?;
    if run_result(&session.route, &run_id)
        .await?
        .run_state()
        .is_terminal()
    {
        return Err("boundary cancellation ended the run before its tool batch finished".into());
    }
    case.subject
        .release_tool_call(&first.arguments)
        .await
        .map_err(|e| e.to_string())?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let result = run_result(&session.route, &run_id).await?;
    if result.state != states::CANCELLED {
        return Err(format!(
            "boundary cancellation ended {}, not cancelled",
            result.state
        ));
    }
    let (_, messages) = read_all(&session.route).await?;
    for tool in [&first, &second] {
        if holding(&messages, &result_marker(tool)) != 1 {
            return Err(
                "boundary cancellation did not record every tool result exactly once".into(),
            );
        }
        if case.subject.tool_invocations(&tool.arguments).await != 1 {
            return Err(
                "boundary cancellation did not dispatch every batch call exactly once".into(),
            );
        }
    }
    if case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?
        != 1
        || case.subject.tool_invocations(&forbidden.arguments).await != 0
    {
        return Err("boundary cancellation started another model step".into());
    }
    Ok(())
}

async fn unlisted<S: LlmRunnerSubject>(case: &Case<'_, S>, points: &[String]) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let tool = case.mint.call("unlisted-cancel-intact-step");
    let session = case
        .open(
            "cancel-unlisted",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool)]),
                    ScriptedTurn::new(vec![ScriptedPart::Text(
                        "unlisted cancel left the run intact".into(),
                    )]),
                ],
            },
        )
        .await?;
    let (reply, run_id) = start_held(case, &session).await?;
    let mut at = case.mint.next("unlisted-cancel-at");
    while points.contains(&at) {
        at = case.mint.next("unlisted-cancel-at");
    }
    let before_head = head(&session.route).await?;
    let before_result = run_result(&session.route, &run_id).await?;
    let before_messages = read_all(&session.route).await?.1;
    let error = expect_error(
        "unlisted cancellation point",
        call(
            &session.route,
            ops::RUN_CANCEL,
            json!({"run_id":run_id, "at":at}),
        )
        .await?,
    )?;
    expect_field(
        "unlisted cancellation point",
        &error,
        errors::INVALID_PARAMS,
        "at",
    )?;
    if head(&session.route).await? != before_head
        || run_result(&session.route, &run_id).await? != before_result
        || read_all(&session.route).await?.1 != before_messages
    {
        return Err("an unlisted cancellation point changed the run or transcript".into());
    }
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    if run_result(&session.route, &run_id).await?.state != states::COMPLETED {
        return Err("an unlisted cancellation point changed the eventual terminal".into());
    }
    Ok(())
}

async fn escalation<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let session = case
        .open(
            "cancel-escalation",
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                    "held model must not need to finish".into(),
                )])],
            },
        )
        .await?;
    let (reply, run_id) = start_held(case, &session).await?;
    cancel(&session.route, &run_id, cancel_at::STEP_BOUNDARY).await?;
    if run_result(&session.route, &run_id)
        .await?
        .run_state()
        .is_terminal()
    {
        return Err("boundary cancellation did not wait for the current model step".into());
    }
    cancel(&session.route, &run_id, cancel_at::NOW).await?;
    // Keep the model held while waiting: releasing it would let a runner that
    // ignored escalation pass by cancelling at the original boundary.
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    if run_result(&session.route, &run_id).await?.state != states::CANCELLED {
        return Err("escalation to now did not end cancelled".into());
    }
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    if run_result(&session.route, &run_id).await?.state != states::CANCELLED {
        return Err("the released answer replaced the cancelled terminal".into());
    }
    Ok(())
}
