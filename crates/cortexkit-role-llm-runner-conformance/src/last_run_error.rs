//! Optional retry metadata must describe the same failed run as run.status.

use cortexkit_role_llm_runner::{
    ops,
    read::HeadMeta,
    run::{RunError, RunState, RunStatus, RunStatusRequest},
    send::SendReply,
};
use serde_json::json;

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_ok, head, send_params, wait_run_end, MAX_POLLS},
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedTurn},
};

async fn send_once<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    first: bool,
) -> Result<SendReply, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let params = send_params(
        case.subject,
        &session.name,
        first,
        &case.mint.next("head-error-prompt"),
        &case.mint.next("head-error-send"),
        Some("queue"),
    );
    decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )
}

async fn send_terminal<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    first: bool,
) -> Result<(String, RunStatus), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let sent = send_once(case, session, first).await?;
    let run_id = sent
        .run_id
        .ok_or("session.send omitted run_id with run_ops")?;
    // Poll the independent run record so malformed head state cannot hide a
    // violation by making the suite wait forever for a terminal head.
    for _ in 0..MAX_POLLS {
        let status: RunStatus = decode(
            "run.status",
            expect_ok(
                "run.status",
                call(
                    &session.route,
                    ops::RUN_STATUS,
                    json!(RunStatusRequest::new().with_run_id(&run_id)),
                )
                .await?,
            )?,
        )?;
        if RunState::parse(&status.state).is_terminal() {
            return Ok((run_id, status));
        }
        case.subject.pause().await;
    }
    Err(format!(
        "the scripted run did not end within {MAX_POLLS} polls"
    ))
}

fn check(name: &str, head: &HeadMeta, status: Option<(&str, &RunStatus)>) -> Result<(), String> {
    let Some(error) = head.last_run_error() else {
        return Ok(());
    };
    match name {
        "head_last_run_error_only_on_error" => {
            if head
                .last_run_state
                .as_ref()
                .is_some_and(|run| run.state != "error")
            {
                return Err(format!(
                    "last_run_error present on non-error state: {:?}",
                    head.last_run_state
                ));
            }
        }
        "head_last_run_error_requires_last_run_state" => {
            if head.last_run_state.is_none() {
                return Err("last_run_error present without last_run_state".into());
            }
        }
        "head_last_run_error_matches_run_status" => {
            let (run_id, status) = status.ok_or("last_run_error present before any run")?;
            if head
                .last_run_state
                .as_ref()
                .is_some_and(|run| run.run_id != run_id)
            {
                return Err("last_run_error describes a different run than run.status".into());
            }
            let expected = status
                .error
                .as_ref()
                .ok_or("last_run_error present without RunError on run.status")?;
            let delay = expected
                .retry_after_secs
                .filter(|n| n.is_finite() && *n >= 0.0);
            if error.class != expected.class
                || error.retry_after_secs() != delay
                || error.resets_at_ms() != expected.resets_at_ms
            {
                return Err(format!(
                    "last_run_error {error:?} differs from run.status RunError {expected:?}"
                ));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let supports_failure = case.subject.supports_model_failure_fixture();
    let matches_status = name == "head_last_run_error_matches_run_status";
    if matches_status && !supports_failure {
        return Ok(Ending::Inapplicable(
            "adapter cannot script a terminal model failure".into(),
        ));
    }
    let mut reported = false;
    if !matches_status {
        let ordinary = case
            .open(
                "head-error-ordinary",
                Script {
                    turns: vec![ScriptedTurn::new(vec![
                        case.mint.text("head-error-completed").0,
                    ])],
                },
            )
            .await?;
        let fresh = head(&ordinary.route).await?;
        if name == "head_last_run_error_requires_last_run_state" && fresh.last_run_error().is_some()
        {
            return Err(
                "last_run_error present before any run, without last_run_state to describe".into(),
            );
        }
        check(name, &fresh, None)?;
        reported |= fresh.last_run_error().is_some();
        let sent = send_once(case, &ordinary, true).await?;
        wait_run_end(case.subject, &ordinary.route, case.declared, None, &sent).await?;
        let completed = head(&ordinary.route).await?;
        if completed
            .last_run_state
            .as_ref()
            .is_none_or(|run| run.state != "completed")
        {
            return Err(format!(
                "ordinary turn did not complete: {:?}",
                completed.last_run_state
            ));
        }
        check(name, &completed, None)?;
        reported |= completed.last_run_error().is_some();
    }
    // Fresh and completed heads need no failure fixture or run-status support.
    // Only the additional failed-then-recovered observations depend on them.
    if !supports_failure
        || !case.declared.contains(&Capability::RunOps)
        || !case.declared.contains(&Capability::RunStatus)
    {
        return Ok(if reported {
            Ending::Passed
        } else {
            Ending::Inapplicable("runner does not report head.last_run_error".into())
        });
    }
    let session = case
        .open(
            "head-error",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![]).with_model_failure(
                        RunError::new("transient", "invented provider failure", "rate_limited")
                            .with_retry_after_secs(1.5)
                            .with_resets_at_ms(1790000003000),
                    ),
                    ScriptedTurn::new(vec![case.mint.text("head-error-recovered").0]),
                ],
            },
        )
        .await?;
    check(name, &head(&session.route).await?, None)?;
    let (run_id, status) = send_terminal(case, &session, true).await?;
    if status.state != "error"
        || status.error.as_ref().and_then(|e| e.retry_after_secs) != Some(1.5)
    {
        return Err(format!("model-failure fixture did not produce an error with fractional retry timing: {status:?}"));
    }
    let failed = head(&session.route).await?;
    check(name, &failed, Some((&run_id, &status)))?;
    reported |= failed.last_run_error().is_some();
    let (recovered_id, recovered) = send_terminal(case, &session, false).await?;
    if recovered.state != "completed" {
        return Err(format!("recovery fixture did not complete: {recovered:?}"));
    }
    check(
        name,
        &head(&session.route).await?,
        Some((&recovered_id, &recovered)),
    )?;
    if reported {
        Ok(Ending::Passed)
    } else {
        Ok(Ending::Inapplicable(
            "runner does not report head.last_run_error".into(),
        ))
    }
}
