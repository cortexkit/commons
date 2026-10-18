//! The context-ceiling wait: a run held at its session's ceiling for a
//! compaction view reads as waiting, with a deadline, not as stalled, so a
//! watchdog leaves it alone. Run only when the adapter can hold a session at
//! the ceiling.

use cortexkit_role_llm_runner::{
    describe::RoleDescribe,
    errors::context_ceiling_causes,
    ops,
    run::{states as run_states, RunReason, RunState, RunStatus, RunStatusRequest},
    PROVIDES, RUN_REASON_CONTEXT_CEILING_WAIT,
};
use serde_json::{json, Map};

use crate::{
    cases::Case,
    drive::{call, decode, expect_ok, head, MAX_POLLS},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
};

type Check = Result<(), String>;

/// Whether the live `role.describe` lists `run.status`, so the case may ask
/// it.
async fn serves_run_status<R: RunnerRoute>(route: &R) -> Result<bool, String> {
    let describe: RoleDescribe = decode(
        "role.describe",
        expect_ok(
            "role.describe",
            call(route, ops::ROLE_DESCRIBE, json!({})).await?,
        )?,
    )?;
    Ok(describe
        .major(PROVIDES)
        .is_some_and(|major| major.serves(ops::RUN_STATUS)))
}

async fn status<R: RunnerRoute>(route: &R, run_id: &str) -> Result<RunStatus, String> {
    let params =
        serde_json::to_value(RunStatusRequest::new().with_run_id(run_id)).expect("serializes");
    decode(
        "run.status",
        expect_ok("run.status", call(route, ops::RUN_STATUS, params).await?)?,
    )
}

pub async fn run<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for view_arrived in [true, false] {
        let (answer, _) = case.mint.text("ceiling-answer");
        let session = case
            .open(
                "ceiling-wait",
                Script {
                    turns: vec![ScriptedTurn::new(vec![answer])],
                },
            )
            .await?;
        case.subject
            .hold_context_ceiling(&session.name)
            .await
            .map_err(|e| format!("holding {} at the ceiling: {e}", session.name))?;
        let prompt = case.mint.next("ceiling-prompt");
        let params = crate::tags::params(
            case,
            &session,
            None,
            &prompt,
            &case.mint.next("send"),
            &Map::new(),
        );
        expect_ok(
            "session.send into a session held at the ceiling",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?;
        let waiting = head(&session.route)
            .await?
            .last_run_state
            .ok_or("session.head names no run while the send's run waits at the ceiling")?;
        if RunState::parse(&waiting.state).is_terminal()
            || waiting.reason.as_deref() != Some(RUN_REASON_CONTEXT_CEILING_WAIT)
        {
            return Err(format!(
                "while held at the ceiling, session.head's last run reads {waiting:?}: expected \
                 a non-terminal state with reason {RUN_REASON_CONTEXT_CEILING_WAIT}"
            ));
        }
        let asks_status = serves_run_status(&session.route).await?;
        if asks_status {
            let held = status(&session.route, &waiting.run_id).await?;
            if held.reason != Some(RunReason::ContextCeilingWait)
                || held.run_state().is_terminal()
                || held.deadline_ms.is_none_or(|deadline| deadline == 0)
            {
                return Err(format!(
                    "while held at the ceiling, run.status answered {held:?}: expected an \
                     active run with reason {RUN_REASON_CONTEXT_CEILING_WAIT} and its deadline_ms"
                ));
            }
        }
        case.subject
            .end_context_ceiling_wait(&session.name, view_arrived)
            .await
            .map_err(|e| format!("ending the ceiling wait: {e}"))?;
        let mut ended = None;
        for _ in 0..MAX_POLLS {
            let last = head(&session.route).await?.last_run_state;
            if let Some(last) = last.filter(|last| RunState::parse(&last.state).is_terminal()) {
                ended = Some(last);
                break;
            }
            case.subject.pause().await;
        }
        let ended = ended.ok_or("the run did not end after its ceiling wait")?;
        let expected = if view_arrived {
            run_states::COMPLETED
        } else {
            run_states::CONTEXT_CEILING
        };
        if ended.state != expected || ended.run_id != waiting.run_id {
            return Err(format!(
                "after the ceiling wait ended {}, the run reads {ended:?}, not {expected}",
                if view_arrived {
                    "with a view"
                } else {
                    "without a view"
                }
            ));
        }
        if !view_arrived && asks_status {
            let ended = status(&session.route, &ended.run_id).await?;
            if ended.error.is_some()
                || ended.context_ceiling.as_ref().map(|d| d.cause.as_str())
                    != Some(context_ceiling_causes::VIEW_TIMEOUT)
            {
                return Err(format!(
                    "a run that waited out its ceiling bound answered run.status {ended:?}: \
                     expected context_ceiling with cause view_timeout and no error"
                ));
            }
        }
    }
    Ok(())
}
