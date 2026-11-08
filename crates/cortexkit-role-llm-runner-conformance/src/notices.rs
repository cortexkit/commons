//! Notice observations come from native storage and the scripted provider.

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, read, read_all, wait_run_end},
    subject::{LlmRunnerSubject, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};
use cortexkit_role_llm_runner::{
    capabilities,
    delivery_order::{DeliveryOrder, Mode},
    live::LiveEvent,
    ops,
    read::{NoticeFact, ReadRequest},
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
    let cleared = name.starts_with("cleared_ride_");
    let free_text = name.starts_with("cleared_ride_free_text_");
    let waits = name == "cleared_ride_free_text_waits";
    let switch_omits = name == "cleared_ride_route_switch_omits";
    let switch_persistent = name == "cleared_ride_persistent_never_upgrades";
    let confirmed = !matches!(
        name,
        "cleared_ride_unconfirmed_falls_through" | "cleared_ride_persistent_never_upgrades"
    );
    if cleared && !case.subject.turn_system_cleared_supported() {
        return Ok(Ending::Inapplicable(
            "adapter declares no confirmed clearing route".into(),
        ));
    }
    let describe = case.checked_describe().await?;
    if cleared && !Mode::TurnSystemCleared.is_declared_by(&describe) {
        return Ok(Ending::Inapplicable(
            "turn_system_cleared delivery is not explicitly declared".into(),
        ));
    }
    if !describe.declares(capabilities::NOTICES) {
        return Ok(Ending::Inapplicable(
            "role.describe does not declare notices".into(),
        ));
    }
    if !cortexkit_role_llm_runner::delivery_order::Mode::ToolResult.is_declared_by(&describe) {
        return Ok(Ending::Inapplicable(
            "tool_result delivery is not declared".into(),
        ));
    }
    if cleared {
        case.subject
            .configure_turn_system_cleared_route(None, confirmed)
            .await
            .map_err(|e| e.to_string())?;
    }
    let mut tool = case.mint.call("ride-output");
    tool.hold = true;
    let output = format!("{}\noriginal ☃", case.mint.next("tool-output"));
    tool.result = json!(output);
    let (mut session, sender_route) = crate::delivery_order::sender_session(
        case,
        "notices",
        vec![
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool.clone())]),
            ScriptedTurn::new(vec![case.mint.text("after-ride").0]),
            ScriptedTurn::new(vec![case.mint.text("after-switch").0]),
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
    let text = if cleared && !free_text {
        "Context is 82% full.".to_owned()
    } else {
        format!("{}\nstatus", case.mint.next("ride-text"))
    };
    let mut order = DeliveryOrder::new(if cleared {
        if waits {
            vec![Mode::TurnSystemCleared]
        } else {
            vec![Mode::TurnSystemCleared, Mode::ToolResult]
        }
    } else {
        vec![Mode::ToolResult]
    });
    if cleared && !free_text {
        order = order.with_template(crate::delivery_order::template_use(json!(82)));
    }
    let mut ride_params = crate::send_helpers::params(
        case,
        &session,
        None,
        &text,
        &send_id,
        &order.to_send_members(),
    );
    if order.template.is_some() {
        ride_params.as_object_mut().unwrap().remove("prompt");
    }
    expect_ok(
        "ride send",
        call(&sender_route, ops::SESSION_SEND, ride_params.clone()).await?,
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
        .notices_stored_output(case.handle.inner(), &session.name, &tool.arguments)
        .await
        .map_err(|e| e.to_string())?;
    let (_, messages) = read_all(&session.route).await?;
    let after = head(&session.route).await?;
    let mut expected_fact =
        NoticeFact::new(part, &send_id, &sender, &text).with_call_key(&call_key);
    if cleared {
        let requests = case
            .subject
            .model_requests(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        if requests.is_empty()
            || requests
                .iter()
                .any(|r| r.system_text.is_none() || r.cleared_system_text.is_none())
        {
            return Err(
                "cleared-ride cases require observed system texts and clearing markers".into(),
            );
        }
        if free_text
            && requests.iter().any(|r| {
                r.system_text
                    .as_ref()
                    .unwrap()
                    .iter()
                    .chain(r.cleared_system_text.as_ref().unwrap())
                    .any(|t| t.contains(&text))
            })
        {
            return Err("free text reached system authority".into());
        }
        if waits {
            let receipt: SendReply = decode(
                "pending free text",
                expect_ok(
                    "pending free text",
                    call(&sender_route, ops::SESSION_SEND, ride_params).await?,
                )?,
            )?;
            if receipt.state != "pending"
                || requests
                    .iter()
                    .any(|r| r.rendered.iter().any(|t| t.contains(&text)))
            {
                return Err("free-text cleared-only send did not stay pending".into());
            }
            return Ok(Ending::Passed);
        }
        let delivered_cleared = confirmed && !free_text;
        expected_fact.delivered_as = if delivered_cleared {
            "turn_system_cleared"
        } else {
            "tool_result"
        }
        .into();
        if stored_output != output
            || before.history_version != after.history_version
            || messages.iter().any(|m| {
                m.delivered_as.as_ref().is_some_and(|d| {
                    d.mode == "turn_system_cleared" || (d.mode == "tool_result" && m.mid != mid)
                })
            })
        {
            return Err(
                "cleared ride changed stored output/history or created a transcript entry".into(),
            );
        }
        let page = read(
            &session.route,
            &ReadRequest::range(0)
                .with_limit(100)
                .with_include_display_facts(true),
        )
        .await?;
        if page
            .messages
            .iter()
            .find(|m| m.mid == mid)
            .map(|m| &m.notices)
            != Some(&vec![expected_fact.clone()])
        {
            return Err(format!(
                "wrong recorded cleared-ride fact: expected {expected_fact:?}"
            ));
        }
        let display = expect_ok(
            "cleared ride display",
            call(
                &session.route,
                ops::SESSION_HISTORY,
                json!({"from_ordinal":1,"limit":100,"form":"display"}),
            )
            .await?,
        )?;
        if !display["messages"].as_array().is_some_and(|rows| {
            rows.iter()
                .any(|row| row["mid"] == mid && row["row"]["notices"] == json!([expected_fact]))
        }) {
            return Err("wrong cleared-ride display fact".into());
        }
        let events = crate::live::stream(&session.route, SubscribeFrom::Start).await?;
        let matching: Vec<_> = events
            .iter()
            .filter_map(|f| match &f.event {
                Some(LiveEvent::NoticeDelivered(e)) if e.send_id == send_id => Some(e),
                _ => None,
            })
            .collect();
        if matching.len() != 1
            || matching[0].mid != mid
            || matching[0].part != part
            || matching[0].call_key != call_key
            || matching[0].text != text
            || matching[0].sender != sender
            || matching[0].delivered_as.as_deref().unwrap_or("tool_result")
                != expected_fact.delivered_as
        {
            return Err("wrong cleared-ride notice_delivered event".into());
        }
        if delivered_cleared {
            if requests
                .iter()
                .any(|r| r.rendered.contains(&format!("{output}\n\n{text}")))
            {
                return Err("cleared ride was also appended as tool_result".into());
            }
            if !requests
                .iter()
                .any(|r| r.cleared_system_text.as_ref().unwrap().contains(&text))
            {
                return Err("confirmed route did not receive a cleared system message".into());
            }
        } else if requests.iter().any(|r| {
            r.system_text
                .as_ref()
                .unwrap()
                .iter()
                .any(|t| t.contains(&text))
        }) {
            return Err("fallback ride reached system authority".into());
        } else if !requests
            .iter()
            .any(|r| r.rendered.contains(&format!("{output}\n\n{text}")))
        {
            return Err("fallback ride did not append as tool_result".into());
        }
        if switch_omits || switch_persistent {
            case.subject
                .configure_turn_system_cleared_route(Some(&session.name), switch_persistent)
                .await
                .map_err(|e| e.to_string())?;
            crate::send_helpers::send_and_wait(
                case,
                &mut session,
                None,
                "after route switch",
                &Map::new(),
            )
            .await?;
            let requests = case
                .subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            let last = requests
                .last()
                .ok_or("route switch made no model request")?;
            let system = last
                .system_text
                .as_ref()
                .ok_or("route switch omitted system observation")?;
            let cleared_text = last
                .cleared_system_text
                .as_ref()
                .ok_or("route switch omitted clearing observation")?;
            if system.iter().any(|t| t.contains(&text))
                || cleared_text.iter().any(|t| t.contains(&text))
            {
                return Err("route switch rendered a ride in a forbidden system form".into());
            }
            if switch_omits && last.rendered.iter().any(|t| t.contains(&text)) {
                return Err("cleared ride was not omitted on a non-clearing route".into());
            }
            if switch_persistent && !last.rendered.contains(&format!("{output}\n\n{text}")) {
                return Err("persistent ride changed after switching to a clearing route".into());
            }
            let page = read(
                &session.route,
                &ReadRequest::range(0)
                    .with_limit(100)
                    .with_include_display_facts(true),
            )
            .await?;
            if page
                .messages
                .iter()
                .find(|m| m.mid == mid)
                .map(|m| &m.notices)
                != Some(&vec![expected_fact])
            {
                return Err("route switch changed recorded delivered_as".into());
            }
        }
        return Ok(Ending::Passed);
    }
    match name {
        "notices_no_transcript_entry" => {
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
        "notices_stored_output_unchanged" => {
            if stored_output != output {
                return Err(format!(
                    "host stored tool output changed: {stored_output:?}, expected {output:?}"
                ));
            }
        }
        "notices_display_fact" => {
            if messages.iter().any(|message| !message.notices.is_empty()) {
                return Err("default read exposed notices without include_display_facts".into());
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
            if message.notices != [expected_fact.clone()] {
                return Err(format!(
                    "wrong notices fact: {:?}, expected {expected_fact:?}",
                    message.notices
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
            if row["row"]["notices"] != json!([expected_fact]) {
                return Err(format!(
                    "wrong display notices fact: {}",
                    row["row"]["notices"]
                ));
            }
        }
        "notices_live_event" => {
            let events = crate::live::stream(&session.route, SubscribeFrom::Start).await?;
            let matching: Vec<_> = events
                .iter()
                .filter_map(|frame| match &frame.event {
                    Some(LiveEvent::NoticeDelivered(event)) if event.send_id == send_id => {
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
                || matching[0].text != text
            {
                return Err(format!(
                    "missing or incorrect notice_delivered event: {matching:?}; observed {:?}",
                    events
                        .iter()
                        .filter_map(|frame| frame.event.as_ref())
                        .collect::<Vec<_>>()
                ));
            }
        }
        "notices_append_bytes" => {
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
        "notices_tool_result_only_never_system" => {
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
        _ => return Err(format!("unknown notices case {name}")),
    }
    Ok(Ending::Passed)
}
