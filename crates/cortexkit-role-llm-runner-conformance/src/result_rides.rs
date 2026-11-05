//! Ride observations come from native storage and the scripted provider.

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, read, read_all, wait_run_end},
    subject::{LlmRunnerSubject, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};
use cortexkit_role_llm_runner::{
    capabilities,
    live::LiveEvent,
    ops,
    read::{ReadRequest, RideFact},
    send::SendReply,
    subscribe::SubscribeFrom,
};
use serde_json::{json, Map};

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    if !describe.declares(capabilities::RESULT_RIDES) {
        return Ok(Ending::Inapplicable(
            "role.describe does not declare result_rides".into(),
        ));
    }
    if !cortexkit_role_llm_runner::delivery_order::Mode::ToolResult.is_declared_by(&describe) {
        return Ok(Ending::Inapplicable(
            "tool_result delivery is not declared".into(),
        ));
    }
    let mut tool = case.mint.call("ride-output");
    tool.hold = true;
    let output = format!("{}\noriginal ☃", case.mint.next("tool-output"));
    tool.result = json!(output);
    let (session, sender_route) = crate::delivery_order::sender_session(
        case,
        "result-rides",
        vec![
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
            ScriptedTurn::new(vec![case.mint.text("after-ride").0]),
        ],
        false,
        false,
    )
    .await?;
    let prior = head(&session.route).await?.last_run_state;
    let start: SendReply = decode(
        "held tool turn",
        expect_ok(
            "held tool turn",
            call(
                &session.route,
                ops::SESSION_SEND,
                crate::send_helpers::params(
                    case,
                    &session,
                    None,
                    "start tool",
                    &case.mint.next("start"),
                    &Map::new(),
                ),
            )
            .await?,
        )?,
    )?;
    case.subject
        .await_tool_call(&tool.arguments)
        .await
        .map_err(|e| e.to_string())?;
    let before = head(&session.route).await?;
    let sender = case
        .subject
        .plan_sender()
        .ok_or("missing plan sender")?
        .module;
    let send_id = case.mint.next("ride-send");
    let text = format!("{}\nstatus", case.mint.next("ride-text"));
    expect_ok(
        "tool_result-only ride",
        call(
            &sender_route,
            ops::SESSION_SEND,
            crate::send_helpers::params(
                case,
                &session,
                None,
                &text,
                &send_id,
                json!({"delivery":["tool_result"]}).as_object().unwrap(),
            ),
        )
        .await?,
    )?;
    case.subject
        .send_staged(&session.name, &send_id)
        .await
        .map_err(|e| e.to_string())?;
    case.subject
        .release_tool_call(&tool.arguments)
        .await
        .map_err(|e| e.to_string())?;
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        prior.as_ref().map(|run| run.run_id.as_str()),
        &start,
    )
    .await?;
    let (mid, part, call_key, stored_output) = case
        .subject
        .result_rides_stored_output(case.handle.inner(), &session.name, &tool.arguments)
        .await
        .map_err(|e| e.to_string())?;
    let (_, messages) = read_all(&session.route).await?;
    let after = head(&session.route).await?;
    let expected_fact = RideFact::new(part, &send_id, &sender, &text);
    match name {
        "result_rides_no_transcript_entry" => {
            let receipt: SendReply = decode(
                "ride receipt",
                expect_ok(
                    "ride retry",
                    call(
                        &sender_route,
                        ops::SESSION_SEND,
                        crate::send_helpers::params(
                            case,
                            &session,
                            None,
                            &text,
                            &send_id,
                            json!({"delivery":["tool_result"]}).as_object().unwrap(),
                        ),
                    )
                    .await?,
                )?,
            )?;
            if receipt
                .delivered
                .as_ref()
                .and_then(|delivered| delivered.call_key.as_deref())
                != Some(call_key.as_str())
            {
                return Err("ride receipt does not name the carrying call_key".into());
            }
            if messages.iter().any(|message| {
                message
                    .delivered_as
                    .as_ref()
                    .is_some_and(|d| d.mode == "tool_result")
                    || (message.mid != mid
                        && message
                            .message
                            .to_string()
                            .contains(&text.replace('\n', "\\n")))
            }) {
                return Err("a ride created a transcript entry".into());
            }
            if before.history_version != after.history_version {
                return Err("ride delivery moved history_version".into());
            }
        }
        "result_rides_stored_output_unchanged" => {
            if stored_output != output {
                return Err(format!(
                    "host stored tool output changed: {stored_output:?}, expected {output:?}"
                ));
            }
        }
        "result_rides_display_fact" => {
            if messages.iter().any(|message| !message.rides.is_empty()) {
                return Err("default read exposed rides without include_display_facts".into());
            }
            let page = read(
                &session.route,
                &ReadRequest::range(0)
                    .with_limit(100)
                    .with_include_display_facts(true),
            )
            .await?;
            let message = page
                .messages
                .iter()
                .find(|message| message.mid == mid)
                .ok_or("carrying result missing from fact read")?;
            if message.rides != [expected_fact.clone()] {
                return Err(format!(
                    "wrong rides fact: {:?}, expected {expected_fact:?}",
                    message.rides
                ));
            }
            let page = expect_ok(
                "ride display history",
                call(
                    &session.route,
                    ops::SESSION_HISTORY,
                    json!({"from_ordinal":1,"limit":100,"form":"display"}),
                )
                .await?,
            )?;
            let row = page["messages"]
                .as_array()
                .ok_or("display history has no messages")?
                .iter()
                .find(|row| row["mid"] == mid)
                .ok_or("carrying display row missing")?;
            if row["row"]["rides"] != json!([expected_fact]) {
                return Err(format!("wrong display rides fact: {}", row["row"]["rides"]));
            }
        }
        "result_rides_live_event" => {
            let events = crate::live::stream(&session.route, SubscribeFrom::Start).await?;
            let matching: Vec<_> = events
                .iter()
                .filter_map(|frame| match &frame.event {
                    Some(LiveEvent::RideDelivered(event)) if event.send_id == send_id => {
                        Some(event)
                    }
                    _ => None,
                })
                .collect();
            if matching.len() != 1
                || matching[0].mid != mid
                || matching[0].part != part
                || matching[0].call_key != call_key
                || matching[0].sender != sender
            {
                return Err(format!(
                    "missing or incorrect ride_delivered event: {matching:?}; observed {:?}",
                    events
                        .iter()
                        .filter_map(|frame| frame.event.as_ref())
                        .collect::<Vec<_>>()
                ));
            }
        }
        "result_rides_append_bytes" => {
            let expected = format!("{output}\n\n{text}");
            let requests = case
                .subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            if !requests
                .iter()
                .any(|request| request.rendered.contains(&expected))
            {
                return Err(format!(
                    "scripted provider did not receive exact append bytes {expected:?}"
                ));
            }
        }
        "result_rides_tool_result_only_never_system" => {
            let requests = case
                .subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            if requests.is_empty()
                || !requests
                    .iter()
                    .any(|request| request.rendered.iter().any(|part| part.contains(&text)))
            {
                return Err("scripted provider never observed the ride".into());
            }
            if requests.iter().any(|request| request.system_text.is_none()) {
                return Err("scripted provider did not record system-authority texts".into());
            }
            if requests.iter().any(|request| {
                request
                    .system_text
                    .as_ref()
                    .unwrap()
                    .iter()
                    .any(|part| part.contains(&text))
            }) {
                return Err("tool_result-only ride reached system authority".into());
            }
        }
        _ => return Err(format!("unknown result_rides case {name}")),
    }
    Ok(Ending::Passed)
}
