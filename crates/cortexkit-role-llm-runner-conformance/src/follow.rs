//! Following a queued send (`CONTRACT.md` §9, §9.7 and §9.8 in
//! `cortexkit-role-llm-runner`): once a send answered `pending` is
//! delivered, a re-send of its `send_id`, and `send.status` on a runner that
//! declares `send_status`, answer how and into which run, and keep answering
//! it across a restart.
//!
//! The two restart cases run on a state root of their own, stopped and
//! reopened with the harness restart hook (no kill point is needed), because
//! the suite's shared runner is never restarted.

use std::{collections::BTreeSet, path::Path};

use cortexkit_role_harness::CrashDriver;
use cortexkit_role_llm_runner::{
    capabilities as groups,
    delivery_order::{modes, DeliveryOrder, Mode},
    errors, ops,
    run::RunState,
    send::{DeliveredAs, SendReply, SendState},
    send_status::{SendStatusReply, SendStatusRequest},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Session},
    delivery_order::delivered_once,
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, read_all,
        wait_run_end, Mint, MAX_POLLS,
    },
    route::{Reply, RunnerRoute},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    tags::{params, require_declared, send_and_wait},
};

type Check = Result<(), String>;

/// The case that follows a delivered steer by re-sending it.
pub const RESEND_CASE: &str = "delivery_order_steer_receipt_durable_across_restart";

/// The case that follows the same steer through `send.status`.
pub const STATUS_CASE: &str = "send_status_answers_as_a_resend_after_restart";

/// `send.status` of `send_id`, answered or refused.
async fn status_raw<R: RunnerRoute>(route: &R, send_id: &str) -> Result<Reply, String> {
    call(
        route,
        ops::SEND_STATUS,
        serde_json::to_value(SendStatusRequest::new(send_id)).expect("serializes"),
    )
    .await
}

async fn status<R: RunnerRoute>(route: &R, send_id: &str) -> Result<SendStatusReply, String> {
    decode(
        "send.status",
        expect_ok(
            &format!("send.status of {send_id}"),
            status_raw(route, send_id).await?,
        )?,
    )
}

async fn resend<R: RunnerRoute>(
    route: &R,
    what: &str,
    params: &Value,
) -> Result<SendReply, String> {
    decode(
        what,
        expect_ok(what, call(route, ops::SESSION_SEND, params.clone()).await?)?,
    )
}

/// `reply` says the send went into run `joined` as a steer: `delivered:
/// {as: step, ref}` and `run_id` that run.
fn expect_step(what: &str, reply: &SendReply, joined: &str) -> Check {
    let step = reply
        .delivered
        .as_ref()
        .is_some_and(|d| d.r#as() == DeliveredAs::Step && d.r#ref.is_some());
    if !step {
        return Err(format!(
            "{what} answered state {} with delivered {:?}: a queued send delivered as a steer \
             answers delivered {{as: step, ref}}, durably, never pending or absent",
            reply.state, reply.delivered
        ));
    }
    if reply.run_id.as_deref() != Some(joined) {
        return Err(format!(
            "{what} answered run_id {:?}, not {joined}, the run the steer joined",
            reply.run_id
        ));
    }
    Ok(())
}

/// `later` carries the same receipt and run as `earlier`: a delivered
/// send's answer never changes.
fn expect_same_receipt(what: &str, earlier: &SendReply, later: &SendReply) -> Check {
    if (&later.delivered, &later.run_id) != (&earlier.delivered, &earlier.run_id) {
        return Err(format!(
            "{what} answered delivered {:?} with run_id {:?}, but the send was answered \
             delivered {:?} with run_id {:?} before: a delivered send's answer is final",
            later.delivered, later.run_id, earlier.delivered, earlier.run_id
        ));
    }
    Ok(())
}

/// Wait until run `run_id` has a terminal state on `session.head`.
async fn wait_ended<S>(subject: &S, route: &S::Route, run_id: &str) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for _ in 0..MAX_POLLS {
        let last = head(route).await?.last_run_state;
        if last
            .as_ref()
            .is_some_and(|l| l.run_id == run_id && RunState::parse(&l.state).is_terminal())
        {
            return Ok(());
        }
        subject.pause().await;
    }
    Err(format!("run {run_id} did not end within {MAX_POLLS} polls"))
}

/// What the restart cases carry across the restart.
struct Followed {
    session: String,
    /// The queued send's params, re-sent unchanged.
    params: Value,
    send_id: String,
    /// The run the steer joined.
    joined: String,
    /// The re-send's answer before the restart.
    before: SendReply,
}

/// Queue a `[steer]` list behind a run held on a tool call, release the
/// call, and check the steer was delivered into that run and is answered so.
async fn queue_and_deliver<S>(case: &Case<'_, S>, via_status: bool) -> Result<Followed, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut held = case.mint.call("follow-held");
    held.hold = true;
    let mut session: Session<S::Route> = case
        .open(
            "follow",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("follow-opening-answer").0]),
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
                    ScriptedTurn::new(vec![case.mint.text("follow-after-steer").0]),
                ],
            },
        )
        .await?;
    require_declared(&session.route, groups::DELIVERY_ORDER, "a list in delivery").await?;
    if via_status {
        require_declared(&session.route, groups::SEND_STATUS, ops::SEND_STATUS).await?;
    }
    let opening = case.mint.next("follow-opening");
    send_and_wait(case, &mut session, None, &opening, &Map::new()).await?;
    expect_ok(
        "the owner's turn with a held call",
        call(
            &session.route,
            ops::SESSION_SEND,
            params(
                case,
                &session,
                None,
                &case.mint.next("follow-held-turn"),
                &case.mint.next("send"),
                &Map::new(),
            ),
        )
        .await?,
    )?;
    case.subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("waiting for the held call: {e}"))?;
    let joined = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id)
        .ok_or("session.head names no run while a call is held")?;
    let marker = case.mint.next("queued-steer");
    let send_id = case.mint.next("send");
    let members = DeliveryOrder::new(vec![Mode::Steer]).to_send_members();
    let send = params(case, &session, None, &marker, &send_id, &members);
    let first = resend(&session.route, "a [steer] list behind a held run", &send).await?;
    if first.send_state() != SendState::Pending || first.submission_id.is_none() {
        return Err(format!(
            "a [steer] list sent while a run waits on a held call answered state {} with \
             submission_id {:?}: it waits, queued, for the run's next step, answering pending \
             with its submission_id",
            first.state, first.submission_id
        ));
    }
    case.subject
        .release_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("releasing the held call: {e}"))?;
    wait_ended(case.subject, &session.route, &joined).await?;
    let (_, messages) = read_all(&session.route).await?;
    delivered_once(&messages, &marker, modes::STEER)?;
    let before = resend(&session.route, "a re-send of the delivered steer", &send).await?;
    expect_step("a re-send of the steer once delivered", &before, &joined)?;
    if via_status {
        let answered = status(&session.route, &send_id).await?;
        if answered != before {
            return Err(format!(
                "send.status answered {answered:?}, but a re-send of the same send_id answered \
                 {before:?}: send.status answers exactly what a re-send would"
            ));
        }
    }
    Ok(Followed {
        session: session.name,
        params: send,
        send_id,
        joined,
        before,
    })
}

/// Deliver a queued steer on a root of its own, stop the runner, reopen it
/// with the harness restart hook, and follow the send again: by re-sending
/// it, or through `send.status` for [`STATUS_CASE`].
pub async fn restart<S>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
    name: &str,
) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let via_status = name == STATUS_CASE;
    let root = work_dir.join(if via_status {
        "send-status-restart"
    } else {
        "steer-receipt-restart"
    });
    std::fs::create_dir_all(&root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let handle = driver.spawn(&root).await.map_err(|e| e.to_string())?;
    let followed = queue_and_deliver(
        &Case {
            subject,
            handle: &handle,
            declared,
            mint,
        },
        via_status,
    )
    .await?;
    // Harness handles stop their module on drop, with all its memory.
    drop(handle);
    let handle = subject
        .restart(&root)
        .await
        .map_err(|e| format!("restarting: {e}"))?;
    let route = subject
        .session_route(&handle, &followed.session, &subject.owner_stamp())
        .await
        .map_err(|e| format!("reopening {}: {e}", followed.session))?;
    if via_status {
        let answered = status(&route, &followed.send_id).await?;
        let what = "send.status of the steer after the restart";
        if answered.send_state() == SendState::Pending {
            return Err(format!(
                "{what} answered {answered:?}: the send was delivered as a steer before the \
                 restart, and a delivered send's answer is durable, never pending again"
            ));
        }
        expect_step(what, &answered, &followed.joined)?;
        expect_same_receipt(what, &followed.before, &answered)?;
        let again = resend(&route, "a re-send after the restart", &followed.params).await?;
        if again != answered {
            return Err(format!(
                "{what} answered {answered:?}, but a re-send of the same send_id then answered \
                 {again:?}: send.status answers exactly what a re-send would"
            ));
        }
        Ok(())
    } else {
        let what = "a re-send of the steer after the restart";
        let again = resend(&route, what, &followed.params).await?;
        expect_step(what, &again, &followed.joined)?;
        expect_same_receipt(what, &followed.before, &again)
    }
}

/// `send.status` of a settled send answers what its re-send answers; a
/// `send.status` naming a `send_id` never sent is refused `unknown_send`; a
/// `send.status` carrying an unknown field is refused naming that field.
pub async fn unknown_send<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "send-status",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("status-opening-answer").0]),
                    ScriptedTurn::new(vec![case.mint.text("status-answer").0]),
                ],
            },
        )
        .await?;
    require_declared(&session.route, groups::SEND_STATUS, ops::SEND_STATUS).await?;
    let opening = case.mint.next("status-opening");
    send_and_wait(case, &mut session, None, &opening, &Map::new()).await?;
    let send_id = case.mint.next("send");
    let send = params(
        case,
        &session,
        None,
        &case.mint.next("status-prompt"),
        &send_id,
        &Map::new(),
    );
    let before = head(&session.route).await?.last_run_state.map(|l| l.run_id);
    let reply = resend(&session.route, "a send", &send).await?;
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    let resent = resend(&session.route, "a re-send of a settled send", &send).await?;
    let answered = status(&session.route, &send_id).await?;
    if answered != resent {
        return Err(format!(
            "send.status of a settled send answered {answered:?}, but its re-send answered \
             {resent:?}: send.status answers exactly what a re-send would"
        ));
    }
    let what = "send.status of a send_id the session has no record of";
    let error = expect_error(
        what,
        status_raw(&session.route, &case.mint.next("never-sent")).await?,
    )?;
    expect_code(what, &error, errors::UNKNOWN_SEND)?;
    let what = "send.status carrying an unknown field";
    let error = expect_error(
        what,
        call(
            &session.route,
            ops::SEND_STATUS,
            json!({ "send_id": send_id, "sendid": send_id }),
        )
        .await?,
    )?;
    expect_field(what, &error, errors::INVALID_PARAMS, "sendid")
}
