//! Held tool-provider replies, followed by separately published late entries.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    describe::{check_describe, RoleDescribe},
    ops,
    run::{RunStatus, RunStatusRequest},
    subscribe::{kinds, SubscribeEvent, SubscribeFrom, SubscribeRequest},
};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, run_result, wait_run_end, MAX_POLLS},
    route::{RunnerRoute, SubscribeOutcome},
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

async fn controls<R: RunnerRoute>(
    route: &R,
    from: &mut SubscribeFrom,
) -> Result<Vec<SubscribeEvent>, String> {
    let events = route
        .subscribe(serde_json::to_value(SubscribeRequest::new(from.clone())).unwrap())
        .await
        .map_err(|e| format!("session.subscribe: the route failed: {}", e.message))?;
    let SubscribeOutcome::Events(events) = events else {
        return Err(format!("session.subscribe was refused: {events:?}"));
    };
    let mut controls = Vec::new();
    for raw in events {
        let frame: SubscribeEvent = decode("a stream frame", raw)?;
        let held = frame
            .held_calls()
            .map_err(|e| format!("a stream event's held_calls does not decode: {e}"))?;
        if held.is_empty() && frame.body.contains_key("held_calls") {
            return Err("a stream event carries held_calls: [] instead of omitting it".into());
        }
        if frame.kind != kinds::CONTROL {
            continue;
        }
        let cursor = frame
            .cursor
            .clone()
            .ok_or("a held-call control event has no durable cursor")?;
        *from = SubscribeFrom::After(cursor);
        controls.push(frame);
    }
    Ok(controls)
}

async fn waiting_events<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    run_id: &str,
    from: &mut SubscribeFrom,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    for _ in 0..MAX_POLLS {
        let during = status(route, run_id).await?;
        let mut listed = false;
        for frame in controls(route, from).await? {
            let held = frame.held_calls().expect("controls decoded held_calls");
            // Other durable events may interleave with status events. Only a
            // non-empty list identifies a held-call status in the open schema.
            if held.is_empty() {
                continue;
            }
            if held != during.held_calls() {
                // The contract doesn't order the event against the status the
                // runner serves, so the event may have arrived after the
                // status read above. Read the status again before failing.
                let now = status(route, run_id).await?;
                if held != now.held_calls() {
                    return Err(format!(
                        "a control event's held_calls differs from run.status: {held:?} != {:?}",
                        now.held_calls()
                    ));
                }
            }
            listed = true;
        }
        if listed {
            return Ok(());
        }
        case.subject.pause().await;
    }
    Err("no control event listed held_calls while waiting".into())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let event_case = name == "held_calls_status_event_matches_run_status";
    let session = case.open("held-calls", Script::default()).await?;
    let raw = expect_ok(
        "role.describe",
        call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
    )?;
    if event_case {
        let describe: RoleDescribe = decode("role.describe", raw.clone())?;
        for group in [
            groups::HELD_CALLS,
            groups::RUN_STATUS,
            groups::QUEUE,
            groups::TRANSCRIPT_READS,
            groups::RUN_OPS,
            groups::STREAMING,
        ] {
            if !describe.declares(group) {
                return Ok(Ending::Inapplicable(format!(
                    "the runner does not declare {group}"
                )));
            }
        }
    }
    let describe =
        check_describe(&raw).map_err(|problems| format!("invalid role.describe: {problems:?}"))?;
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
    // The route closes subscriptions after a quiet window. Attach before
    // sending, then resume from its durable cursor so no status can be missed.
    let mut from = SubscribeFrom::Start;
    if event_case {
        controls(&session.route, &mut from).await?;
    }
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
    if event_case {
        waiting = waiting_events(case, &session.route, &run_id, &mut from).await;
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
    if event_case {
        // `from` is the last position read before publishing the late result,
        // not the first unrelated control event that omits the list.
        for frame in controls(&session.route, &mut from).await? {
            if frame
                .held_calls()
                .expect("controls decoded held_calls")
                .iter()
                .any(|entry| entry.call_key() == call_key)
            {
                return Err(
                    "a control event lists the held call after applying its late result".into(),
                );
            }
        }
        return Ok(Ending::Passed);
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
