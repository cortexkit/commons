//! Sender bindings are display facts available when a user message starts.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    delivery_order::{Mode, PlanSender},
    display::{DisplayPage, DisplayPart, DisplayRow},
    history::{HistoryForm, HistoryRequest},
    live::LiveEvent,
    ops,
    send::SendReply,
    subscribe::SubscribeFrom,
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, wait_run_end},
    live::stream,
    route::{Reply, RouteFailure, RunnerRoute},
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    tags::send_and_wait,
};

fn row(reply: Reply, mid: &str) -> Result<DisplayRow, String> {
    let page: DisplayPage = decode(
        "message_started display row",
        expect_ok("message_started display row", reply)?,
    )?;
    let entry = page
        .messages
        .into_iter()
        .next()
        .ok_or_else(|| format!("message_started {mid} has no readable display row"))?;
    if entry.mid != mid {
        return Err(format!("read by mid {mid} returned {}", entry.mid));
    }
    Ok(entry.row)
}

fn names_sender(row: &DisplayRow, sender: &str, client_id: &str) -> Result<(), String> {
    if row.sender.as_deref() != Some(sender) {
        return Err(format!(
            "sender is {:?}, expected the sending module {sender}",
            row.sender
        ));
    }
    if row.client_message_id.as_deref() != Some(client_id) {
        return Err(format!(
            "client_message_id is {:?}, expected {client_id}",
            row.client_message_id
        ));
    }
    Ok(())
}

fn contains(row: &DisplayRow, text: &str) -> bool {
    row.parts
        .iter()
        .any(|part| matches!(part, DisplayPart::Text(part) if part.text == text))
}

async fn observe<R, F>(
    route: &R,
    operation: F,
    observed: &mut Vec<(Value, Reply)>,
    unavailable: &mut Vec<String>,
) -> Result<(SendReply, bool), String>
where
    R: RunnerRoute,
    F: std::future::Future<Output = Result<Reply, RouteFailure>> + Send,
{
    let mut operation = Box::pin(operation);
    let (reply, timing) = match route
        .observe_message_started(operation.as_mut())
        .await
        .map_err(|e| e.message)?
    {
        Some((reply, rows)) => {
            observed.extend(rows);
            (reply, true)
        }
        None => {
            let reason =
                "message_senders_timing_at_message_started: adapter has no live observation hook";
            if !unavailable.iter().any(|entry| entry == reason) {
                unavailable.push(reason.into());
            }
            (operation.await.map_err(|e| e.message)?, false)
        }
    };
    Ok((
        decode("sender send", expect_ok("sender send", reply)?)?,
        timing,
    ))
}

pub async fn run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    for group in [
        groups::MESSAGE_SENDERS,
        groups::HISTORY_DISPLAY,
        groups::LIVE_EVENTS,
        groups::DELIVERY_ORDER,
        groups::QUEUE,
        groups::CLIENT_MESSAGE_ID,
    ] {
        if !describe.declares(group) {
            return Ok(Ending::Inapplicable(format!(
                "runner does not declare {group}"
            )));
        }
    }
    let Some([a, b]) = case.subject.head_sender_identities() else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no named-principal routes (head_sender_identities)".into(),
        ));
    };
    if a.module == b.module || a.stamp == b.stamp {
        return Err("sender fixture identities must be distinct".into());
    }
    if !Mode::IdleStart.is_declared_by(&describe) {
        return Ok(Ending::Inapplicable(
            "runner does not declare the idle_start list mode".into(),
        ));
    }
    let (answer, answer_marker) = case.mint.text("sender-answer");
    let mut session = case
        .open(
            "message-senders",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("sender-opening-answer").0]),
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(
                        case.mint.call("sender-boundary"),
                    )]),
                    ScriptedTurn::new(vec![answer]),
                ],
            },
        )
        .await?;
    let members = json!({"senders": [PlanSender::new(&b.module).with_idle_start()]});
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, members.as_object().unwrap())
        .ok_or("adapter supplies sender identities but no plan carrying senders")?;
    send_and_wait(
        case,
        &mut session,
        Some(&fields),
        &case.mint.next("sender-opening"),
        &Map::new(),
    )
    .await?;
    let sender = case
        .subject
        .session_route(case.handle.inner(), &session.name, &b.stamp)
        .await
        .map_err(|e| e.to_string())?;
    let held = Mode::Steer.is_declared_by(&describe)
        && case
            .subject
            .hold_next_model_step(&session.name)
            .await
            .is_ok();
    let mut wanted = Vec::new();
    let mut observed = Vec::new();
    let mut unavailable = Vec::new();
    let mut released = false;
    let checked = async {
        let prompt = case.mint.next("sender-start");
        let client_id = case.mint.next("sender-client-start");
        let params = send_params(case, &session.name, &prompt, &client_id, "idle_start");
        let (started, timing) = observe(
            &session.route,
            sender.request(ops::SESSION_SEND, params),
            &mut observed,
            &mut unavailable,
        ).await?;
        wanted.push((prompt, client_id, timing));
        if held {
            case.subject.await_model_step(&session.name).await.map_err(|e| e.to_string())?;
            let prompt = case.mint.next("sender-join");
            let client_id = case.mint.next("sender-client-join");
            let params = send_params(case, &session.name, &prompt, &client_id, "steer");
            // Release even if the joining send fails, so a rejected send never
            // leaves the adapter's model provider held for later cases.
            let operation = async {
                let reply = sender.request(ops::SESSION_SEND, params.clone()).await;
                let release_result = case.subject.release_model_step(&session.name).await;
                released = true;
                let reply = reply?;
                release_result.map_err(|e| RouteFailure::new(e.to_string()))?;
                let _: SendReply = expect_ok("sender join", reply)
                    .and_then(|value| decode("sender join", value))
                    .map_err(RouteFailure::new)?;
                wait_run_end(case.subject, &session.route, case.declared, None, &started)
                    .await.map_err(RouteFailure::new)?;
                // Ordered sends may answer pending at acceptance. Retry the
                // same keyed send after delivery to inspect its settled receipt.
                sender.request(ops::SESSION_SEND, params).await
            };
            let (joined, timing) = observe(
                &session.route, operation, &mut observed, &mut unavailable,
            ).await?;
            if joined.delivered.as_ref().map(|d| d.r#as.as_str()) != Some("step")
                || (started.run_id.is_some() && joined.run_id != started.run_id) {
                return Err(format!("second sender message did not join the running turn: {joined:?}"));
            }
            wanted.push((prompt, client_id, timing));
        } else {
            unavailable.push("message_senders_join: adapter cannot hold a steerable run open; checked start only".into());
        }
        wait_run_end(case.subject, &session.route, case.declared, None, &started).await?;
        check_rows(&session.route, &wanted, &observed, &b.module, &answer_marker).await
    }.await;
    if held && !released {
        let released = case
            .subject
            .release_model_step(&session.name)
            .await
            .map_err(|e| e.to_string());
        checked?;
        released?;
    } else {
        checked?;
    }
    if unavailable.is_empty() {
        Ok(Ending::Passed)
    } else {
        Ok(Ending::PassedWithInapplicableRule(unavailable.join("; ")))
    }
}

async fn check_rows<R: RunnerRoute>(
    route: &R,
    wanted: &[(String, String, bool)],
    observed: &[(Value, Reply)],
    sender: &str,
    model_reply: &str,
) -> Result<(), String> {
    let mut found = std::collections::BTreeSet::new();
    let mut found_model_reply = false;
    let mut timing_checks = Vec::new();
    for frame in stream(route, SubscribeFrom::Start).await? {
        let Some(LiveEvent::MessageStarted(message)) = frame.event else {
            continue;
        };
        let params = HistoryRequest::from_mid(&message.mid)
            .with_limit(1)
            .with_form(HistoryForm::Display);
        let display = row(
            call(
                route,
                ops::SESSION_HISTORY,
                serde_json::to_value(params).unwrap(),
            )
            .await?,
            &message.mid,
        )?;
        if display.role == "assistant" {
            found_model_reply |= contains(&display, model_reply);
            if display.sender.is_some() {
                return Err(format!(
                    "model reply {} carries sender {:?}",
                    message.mid, display.sender
                ));
            }
        }
        for (prompt, client_id, timing) in wanted {
            if !contains(&display, prompt) {
                continue;
            }
            if display.role != "user" {
                return Err("sender's message is not a user row".into());
            }
            names_sender(&display, sender, client_id)?;
            if !found.insert(prompt.clone()) {
                return Err("duplicate message_started for sender message".into());
            }
            if *timing {
                timing_checks.push((message.mid.clone(), client_id.clone()));
            }
        }
    }
    if found.len() != wanted.len() || !found_model_reply {
        return Err(format!(
            "message_started rows cover {} of {} sends; scripted model reply found: {found_model_reply}",
            found.len(),
            wanted.len()
        ));
    }
    // Validate buffered rows first: a receipt-time failure must not hide a
    // missing sender, wrong client id or attributed model reply after settling.
    for (mid, client_id) in timing_checks {
        let (_, captured) = observed
            .iter()
            .find(|(frame, _)| {
                frame["event"]["type"] == "message_started" && frame["event"]["mid"] == mid
            })
            .ok_or_else(|| {
                format!("message_senders_timing_at_message_started: no live observation for {mid}")
            })?;
        let captured = row(captured.clone(), &mid)
            .map_err(|e| format!("message_senders_timing_at_message_started: {e}"))?;
        names_sender(&captured, sender, &client_id)
            .map_err(|e| format!("message_senders_timing_at_message_started: {e}"))?;
    }
    Ok(())
}

fn send_params<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &str,
    prompt: &str,
    client_id: &str,
    mode: &str,
) -> Value {
    let mut params = case.subject.send_fields(session, false);
    params.extend(json!({"prompt":prompt,"send_id":case.mint.next("sender-send"),"client_message_id":client_id,"delivery":[mode]}).as_object().unwrap().clone());
    Value::Object(params)
}
