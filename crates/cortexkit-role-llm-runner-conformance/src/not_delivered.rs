//! A proven pre-delivery failure is final for its send id, not an unknown outcome.

use cortexkit_role_llm_runner::{
    ops,
    send::{self, SendReply},
};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, read_all, run_result},
    route::RunnerRoute,
    send_helpers::send_and_wait,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedTurn},
};

pub(crate) async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if name == "send_queued_without_own_run_stays_pending" {
        return queued_without_own_run(case).await;
    }
    let reason = match name {
        "send_not_delivered_handover_failed_final" => send::REASON_HANDOVER_FAILED,
        "send_not_delivered_routing_refused_final" => send::REASON_ROUTING_REFUSED,
        _ => unreachable!(),
    };
    let mut session = case
        .open(
            "not-delivered",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("opening-answer").0]),
                    ScriptedTurn::new(vec![case.mint.text("new-id-answer").0]),
                ],
            },
        )
        .await?;
    send_and_wait(
        case,
        &mut session,
        None,
        &case.mint.next("opening"),
        &Map::new(),
    )
    .await?;
    if !case
        .subject
        .prepare_not_delivered_send(&session.name, reason)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(format!(
            "adapter cannot drive a proven {reason} send failure"
        )));
    }
    let before_models = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let prompt = case.mint.next("never-posted");
    let send_id = case.mint.next("send");
    let params = case.params(&session, &prompt, &send_id, Some("queue"));
    let first: SendReply = decode(
        "not_delivered send",
        expect_ok(
            "not_delivered send",
            call(&session.route, ops::SESSION_SEND, params.clone()).await?,
        )?,
    )?;
    if first.state != send::states::NOT_DELIVERED
        || first.reason.as_deref() != Some(reason)
        || first.delivered.is_some()
    {
        return Err(format!(
            "expected not_delivered with reason {reason}, got {first:?}"
        ));
    }
    let terminal_head = head(&session.route).await?.last_run_state;
    // The first failure may have written local refusal bookkeeping. Retrying
    // that final answer must not add work or transcript entries of its own.
    let (_, terminal_messages) = read_all(&session.route).await?;
    for _ in 0..2 {
        let retry: SendReply = decode(
            "a terminal send retry",
            expect_ok(
                "a terminal send retry",
                call(&session.route, ops::SESSION_SEND, params.clone()).await?,
            )?,
        )?;
        if retry != first {
            return Err(format!(
                "not_delivered re-send changed its final answer: {first:?} -> {retry:?}"
            ));
        }
        if case.declared.contains(&Capability::SendStatus) {
            let status: SendReply = decode(
                "send_status of a terminal send",
                expect_ok(
                    "send_status of a terminal send",
                    call(
                        &session.route,
                        ops::SESSION_SEND_STATUS,
                        json!({"send_id": send_id}),
                    )
                    .await?,
                )?,
            )?;
            if status != first {
                return Err("send_status changed a not_delivered final answer".into());
            }
        }
    }
    if let Some(run_id) = first
        .run_id
        .as_deref()
        .filter(|_| case.declared.contains(&Capability::RunOps))
    {
        let result = run_result(&session.route, run_id).await?;
        if result.state != "error" || result.error.is_none() {
            return Err(format!(
                "not_delivered run.result must end error carrying its refusal: {result:?}"
            ));
        }
    }
    if case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?
        != before_models
        || read_all(&session.route).await?.1 != terminal_messages
        || head(&session.route).await?.last_run_state != terminal_head
    {
        return Err("not_delivered send or re-send started work or wrote the transcript".into());
    }
    send_and_wait(case, &mut session, None, &prompt, &Map::new()).await?;
    if case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?
        != before_models + 1
    {
        return Err("only the new send_id should reach the model, exactly once".into());
    }
    Ok(Ending::Passed)
}

/// Hold a real idle queue, not an unreachable host: before its own run starts,
/// the send stays pending until the queue can drain, never not_delivered.
/// An allocated run id alone does not mean a run has started.
async fn queued_without_own_run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "parked-queue",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("opening-answer").0]),
                    ScriptedTurn::new(vec![case.mint.text("queued-answer").0]),
                ],
            },
        )
        .await?;
    send_and_wait(case, &mut session, None, "opening", &Map::new()).await?;
    if !case
        .subject
        .hold_retention_queue(&session.name)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "adapter cannot hold an idle queue with no run of its own".into(),
        ));
    }
    let before_models = case
        .subject
        .model_invocations(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let prompt = case.mint.next("parked");
    let send_id = case.mint.next("send");
    let params = case.params(&session, &prompt, &send_id, Some("queue"));
    let held = async {
        for _ in 0..2 {
            let body = expect_ok(
                "held queue",
                call(&session.route, ops::SESSION_SEND, params.clone()).await?,
            )?;
            let reply: SendReply = decode("held queue", body)?;
            if reply.state != send::states::PENDING || reply.submission_id.is_none() {
                return Err(format!("a queued send without its own run must stay pending, never not_delivered: {reply:?}"));
            }
            if case.declared.contains(&Capability::SendStatus) {
                let body = expect_ok(
                    "held queue status",
                    call(&session.route, ops::SESSION_SEND_STATUS, json!({"send_id": send_id})).await?,
                )?;
                let status: SendReply = decode("held queue status", body)?;
                if status.state != send::states::PENDING {
                    return Err(format!("a queued send's status must stay pending without its own run: {status:?}"));
                }
            }
        }
        let models = case.subject.model_invocations(&session.name).await.map_err(|e| e.to_string())?;
        if models != before_models {
            return Err("the held queue started work before release".into());
        }
        Ok(())
    }.await;
    let released = case
        .subject
        .release_retention_queue(&session.name)
        .await
        .map_err(|e| e.to_string());
    held?;
    released?;
    for _ in 0..crate::drive::MAX_POLLS {
        if case
            .subject
            .model_invocations(&session.name)
            .await
            .map_err(|e| e.to_string())?
            > before_models
        {
            return Ok(Ending::Passed);
        }
        case.subject.pause().await;
    }
    Err("the queued send never ran after its session could run again".into())
}

#[cfg(test)]
mod tests;
