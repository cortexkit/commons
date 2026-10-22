//! Held tool-provider replies, followed by separately published late entries.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    describe::check_describe,
    ops,
    run::{RunStatus, RunStatusRequest},
};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, run_result, wait_run_end, MAX_POLLS},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    ScriptedLateEntry,
};

async fn status<R: RunnerRoute>(route: &R, run_id: &str) -> Result<RunStatus, String> {
    decode(
        "run.status",
        expect_ok(
            "run.status",
            call(
                route,
                ops::RUN_STATUS,
                serde_json::to_value(RunStatusRequest::new().with_run_id(run_id)).unwrap(),
            )
            .await?,
        )?,
    )
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open("held-calls", Script::default()).await?;
    let describe = check_describe(&expect_ok(
        "role.describe",
        call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
    )?)
    .map_err(|problems| format!("invalid role.describe: {problems:?}"))?;
    // Discover this additive group by its open string rather than extending
    // the kit's public Capability enum, which older adapters may match on.
    if !describe.declares(groups::HELD_CALLS) {
        return Ok(Ending::Inapplicable(
            "the runner does not declare held_calls".into(),
        ));
    }
    let held = case.mint.call("held-reply");
    let (answer, final_marker) = case.mint.text("after-late-entry");
    case.subject
        .install_script(
            &session.name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
                    ScriptedTurn::new(vec![answer]),
                ],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    case.subject
        .install_held_reply(&session.name, &held.arguments, "consent")
        .await
        .map_err(|e| format!("installing the held-reply provider: {e}"))?;
    let prompt = case.mint.next("held-prompt");
    let params = crate::tags::params(
        case,
        &session,
        None,
        &prompt,
        &case.mint.next("send"),
        &Map::new(),
    );
    let sent = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    let call_key = case
        .subject
        .await_held_reply(&held.arguments)
        .await
        .map_err(|e| format!("observing the provider's held reply: {e}"))?;
    if call_key.is_empty() {
        return Err("the held-reply provider observed no call_key".into());
    }
    let run_id = head(&session.route)
        .await?
        .last_run_state
        .ok_or("session.head names no run after the send")?
        .run_id;
    let listing = name == "held_calls_lists_waiting_call";
    let mut waiting = Ok(());
    if listing {
        waiting = Err("run.status never listed the held call while waiting".into());
        for _ in 0..MAX_POLLS {
            let during = status(&session.route, &run_id).await?;
            if let [entry] = during.held_calls() {
                if entry.call_key() != call_key
                    || entry.tool() != held.tool
                    || entry.provider() != case.subject.tool_provider_module()
                    || entry.reason() != "consent"
                    || entry.held_at_ms() == 0
                    || during.run_state().is_terminal()
                {
                    waiting = Err(format!("run.status lists the wrong held call: {during:?}"));
                } else {
                    waiting = Ok(());
                }
                break;
            }
            case.subject.pause().await;
        }
    }
    let not_started = name == "held_calls_not_started_becomes_tool_result";
    let expired = name == "held_calls_expired_becomes_tool_result";
    let late = if not_started {
        ScriptedLateEntry::not_started("restart_before_dispatch")
    } else if expired {
        ScriptedLateEntry::expired()
    } else {
        ScriptedLateEntry::result(held.result.clone())
    };
    case.subject
        .publish_late_entry(&held.arguments, late)
        .await
        .map_err(|e| format!("publishing the provider's late entry: {e}"))?;
    waiting?;
    wait_run_end(case.subject, &session.route, case.declared, None, &sent).await?;
    if listing {
        return Ok(Ending::Passed);
    }
    let result = run_result(&session.route, &run_id).await?;
    if result.state != "completed"
        || result
            .final_message
            .as_ref()
            .is_none_or(|m| m.text != final_marker)
    {
        return Err(format!(
            "the run did not continue after the late entry: {result:?}"
        ));
    }
    if !not_started && status(&session.route, &run_id).await?.is_held_wait() {
        return Err("run.status kept the held call after applying its late result".into());
    }
    let requests = case
        .subject
        .model_requests(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let rendered = requests
        .last()
        .ok_or("no model request followed the late entry")?
        .rendered
        .join("\n");
    if not_started {
        let text = rendered.to_lowercase();
        if !["never ran", "did not run", "not started", "not_started"]
            .iter()
            .any(|phrase| text.contains(phrase))
        {
            return Err(format!(
                "the model's tool result does not say the call never ran: {rendered}"
            ));
        }
    } else if expired {
        if !rendered.to_lowercase().contains("expired") {
            return Err(format!(
                "the model's tool result does not say the answer expired: {rendered}"
            ));
        }
    } else if !rendered.contains(&crate::drive::result_marker(&held)) {
        return Err("the next model request did not carry the late tool result".into());
    }
    Ok(Ending::Passed)
}
