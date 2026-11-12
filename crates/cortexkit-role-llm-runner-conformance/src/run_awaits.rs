//! One reader-facing wait across the surfaces a runner serves.

use cortexkit_role_llm_runner::{
    awaits::AwaitParty,
    capabilities as groups,
    describe::{check_describe, RoleDescribe},
    errors, ops,
    read::HeadMeta,
    run::{RunError, RunStatus},
    send::SendReply,
    subscribe::{SubscribeEvent, SubscribeFrom, SubscribeRequest},
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_ok, head, run_result, MAX_POLLS},
    route::{RunnerRoute, SubscribeOutcome},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedTurn},
};

async fn status<R: RunnerRoute>(route: &R, run_id: &str) -> Result<RunStatus, String> {
    decode(
        "run.status",
        expect_ok(
            "run.status",
            call(route, ops::RUN_STATUS, json!({"run_id":run_id})).await?,
        )?,
    )
}

async fn send<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
) -> Result<String, String>
where
    S::Route: RunnerRoute,
{
    let params = case.params(session, "continue", &case.mint.next("awaits-send"), None);
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    reply.run_id.ok_or("session.send omitted run_id".into())
}

fn agree(status: &RunStatus, head: &HeadMeta, run_id: &str) -> Result<(), String> {
    let last = head
        .last_run_state
        .as_ref()
        .ok_or("head omitted last_run_state")?;
    let reason = status.reason.clone().map(String::from);
    if last.run_id != run_id
        || last.state != status.state
        || last.reason != reason
        || last.awaits != status.awaits
    {
        return Err(format!(
            "head and run.status disagree for {run_id}: {last:?} != {status:?}"
        ));
    }
    Ok(())
}

async fn stream<R: RunnerRoute>(
    route: &R,
    run_id: &str,
    status: &RunStatus,
    name: &str,
) -> Result<(), String> {
    let params = json!(SubscribeRequest::new(SubscribeFrom::Start));
    let outcome = route.subscribe(params).await.map_err(|e| e.message)?;
    let SubscribeOutcome::Events(events) = outcome else {
        return Err(format!("subscribe refused: {outcome:?}"));
    };
    for raw in events {
        let event: SubscribeEvent = decode("subscribe unit", raw)?;
        if let Some(paused) = event
            .body
            .get("event")
            .filter(|value| value["type"] == "run_paused")
        {
            let paused: cortexkit_role_llm_runner::live::RunEnded =
                decode("run_paused payload", paused.clone())?;
            if name == "run_awaits_same_run_agrees"
                && paused.run_id == run_id
                && status.state == "paused"
                && (paused.awaits != status.awaits
                    || Some(paused.reason) != status.reason.clone().map(String::from))
            {
                return Err("typed run_paused and run.status disagree".into());
            }
        }
        let wait = event
            .awaits()
            .map_err(|e| format!("subscribe awaits: {e}"))?;
        if event.body.get("run_id").and_then(Value::as_str) != Some(run_id) {
            continue;
        }
        // Only units naming the observed lifecycle state can be compared to
        // this stable snapshot; earlier stream states are historical facts.
        if event.body.get("state").and_then(Value::as_str) != Some(status.state.as_str()) {
            continue;
        }
        if name == "run_awaits_same_run_agrees"
            && (wait != status.awaits
                || event.body.get("reason").and_then(Value::as_str)
                    != status.reason.clone().map(String::from).as_deref())
        {
            return Err("subscribe run-status and run.status disagree".into());
        }
        if name == "run_awaits_completed_absent" && wait.is_some() {
            return Err("completed subscribe unit carries awaits".into());
        }
    }
    Ok(())
}

async fn observe<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    run_id: &str,
    name: &str,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let status = status(&session.route, run_id).await?;
    let head = head(&session.route).await?;
    let result = run_result(&session.route, run_id).await?;
    match name {
        "run_awaits_same_run_agrees" => agree(&status, &head, run_id)?,
        "run_awaits_error_mapping" => {
            let expected = status.error.as_ref().map(errors::expected_party);
            if !status.awaits_consistent()
                || !result.awaits_consistent()
                || status.awaits.as_ref().map(|wait| &wait.party) != expected.as_ref()
                || result.awaits.as_ref().map(|wait| &wait.party) != expected.as_ref()
            {
                return Err(format!(
                    "terminal error awaits disagrees with shared mapping: {status:?}, {result:?}"
                ));
            }
        }
        "run_awaits_completed_absent" => {
            if status.awaits.is_some()
                || result.awaits.is_some()
                || head
                    .last_run_state
                    .as_ref()
                    .is_some_and(|run| run.awaits.is_some())
            {
                return Err("completed run carries awaits".into());
            }
        }
        "run_awaits_runner_deadline" => {
            let wait = status
                .awaits
                .as_ref()
                .ok_or("observed context wait omitted awaits")?;
            if wait.party != AwaitParty::Runner || !status.awaits_consistent() {
                return Err(format!("runner-owned wait deadline disagrees: {status:?}"));
            }
        }
        _ => {}
    }
    if case.declared.contains(&Capability::Streaming) {
        stream(&session.route, run_id, &status, name).await?;
    }
    Ok(())
}

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let session = case.open("run-awaits", Script::default()).await?;
    let raw = expect_ok(
        "role.describe",
        call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
    )?;
    let describe: RoleDescribe = decode("role.describe", raw.clone())?;
    if !describe.declares(groups::RUN_AWAITS) {
        return Ok(Ending::Inapplicable(
            "runner does not declare run_awaits".into(),
        ));
    }
    check_describe(&raw).map_err(|e| format!("invalid describe: {e:?}"))?;
    if name == "run_awaits_error_mapping" {
        return errors_case(case, name).await;
    }
    let held = name != "run_awaits_completed_absent"
        && case.declared.contains(&Capability::HoldContextCeiling);
    case.subject
        .install_script(
            &session.name,
            Script {
                turns: vec![ScriptedTurn::new(vec![case.mint.text("awaits-finish").0])],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    if held {
        case.subject
            .hold_context_ceiling(&session.name)
            .await
            .map_err(|e| e.to_string())?;
    }
    let id = send(case, &session).await?;
    for _ in 0..MAX_POLLS {
        let current = status(&session.route, &id).await?;
        if (held
            && current.reason
                == Some(cortexkit_role_llm_runner::run::RunReason::ContextCeilingWait))
            || (!held && current.run_state().is_terminal())
        {
            let checked = observe(case, &session, &id, name).await;
            if held {
                case.subject
                    .end_context_ceiling_wait(&session.name, true)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            checked?;
            if name == "run_awaits_same_run_agrees" && case.subject.supports_model_failure_fixture()
            {
                errors_case(case, name).await?;
            }
            return Ok(Ending::Passed);
        }
        case.subject.pause().await;
    }
    Err("scripted run did not reach its stable wait or ending".into())
}

async fn errors_case<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    if !case.subject.supports_model_failure_fixture() {
        return Ok(Ending::Inapplicable(
            "adapter cannot script terminal model failures".into(),
        ));
    }
    let codes: std::collections::BTreeSet<_> = errors::CODES
        .iter()
        .chain(errors::provider_codes::CODES.iter())
        .copied()
        .chain(["auth_required", "future_error"])
        .collect();
    for code in codes {
        let mut error = RunError::new("invented", "Invented model failure.", code);
        if errors::is_account_wait(code) {
            error = error.with_retry_after_secs(1.5).with_resets_at_ms(u64::MAX);
        }
        let session = case
            .open(
                "awaits-ending",
                Script {
                    turns: vec![ScriptedTurn::new(vec![]).with_model_failure(error)],
                },
            )
            .await?;
        let id = send(case, &session).await?;
        let mut ended = false;
        for _ in 0..MAX_POLLS {
            let current = status(&session.route, &id).await?;
            if current.run_state().is_terminal() {
                if current.state != "error"
                    || current
                        .error
                        .as_ref()
                        .is_none_or(|error| error.provider_code != code)
                {
                    return Err(format!(
                        "model failure fixture did not end error with {code}: {current:?}"
                    ));
                }
                observe(case, &session, &id, name).await?;
                ended = true;
                break;
            }
            case.subject.pause().await;
        }
        if !ended {
            return Err(format!("model failure {code} did not end"));
        }
    }
    Ok(Ending::Passed)
}
