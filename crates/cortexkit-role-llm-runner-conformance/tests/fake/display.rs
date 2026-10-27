//! Display rows and live events of the durable-log fake: the
//! `history_display` and `live_events` groups.
//!
//! Sessions the display and provider-descriptor cases mint
//! (`cf-display` and `cf-descriptor-provider`)
//! keep their bodies in the `anthropic-messages` profile, so their rows come
//! from that profile's projection; every other session keeps the fake's own
//! schema, so the fixtures of the other cases keep their bytes.
use super::*;
use cortexkit_role_llm_runner::{
    display::{
        self as display_rows, DisplayEntry, DisplayPage, DisplayPart, DisplayRow, Limits,
        APP_LIMITS,
    },
    history::{HistoryEntry, HistoryPage},
    live::{
        self, LiveEvent, LiveSubscribeRequest, MessageEvent, RunEnded, RunStarted, TextDelta,
        ToolFinished, ToolInputDelta, ToolStarted,
    },
    subscribe::{kinds, SubscribeEvent},
};

/// Whether `session` keeps its bodies in the `anthropic-messages` profile.
pub(super) fn profiled(session: &str) -> bool {
    session.starts_with("cf-display") || session.starts_with("cf-descriptor-provider")
}

/// The `anthropic-messages` body of an assistant step from the fake's parts.
pub(super) fn anthropic_assistant(parts: &Value) -> Value {
    let blocks: Vec<Value> = parts
        .as_array()
        .into_iter()
        .flatten()
        .map(|part| match part["type"].as_str() {
            Some("reasoning") => {
                json!({ "type": "thinking", "thinking": part["text"], "signature": "fake" })
            }
            Some("tool_call") => json!({ "type": "tool_use", "id": part["id"],
                                         "name": part["tool"], "input": part["arguments"] }),
            _ => json!({ "type": "text", "text": part["text"] }),
        })
        .collect();
    json!({ "role": "assistant", "content": blocks })
}

/// A tool result's text: a string result as it is, any other as compact
/// JSON.
pub(super) fn result_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// The message holding the call `key`, and the name of the tool it calls.
fn call_holder(sess: &Sess, key: &str) -> (String, String) {
    let Some(message) = sess
        .messages
        .iter()
        .find(|m| m.calls.iter().any(|k| k == key))
    else {
        return (String::new(), SCRIPTED_TOOL.to_owned());
    };
    let id = sess
        .calls
        .get(key)
        .map(|c| c.tool_call_id.as_str())
        .unwrap_or_default();
    let parts = message.body["parts"]
        .as_array()
        .or_else(|| message.body["content"].as_array());
    let name = parts
        .into_iter()
        .flatten()
        .find(|part| part["id"] == id)
        .and_then(|part| part["tool"].as_str().or_else(|| part["name"].as_str()))
        .unwrap_or(SCRIPTED_TOOL);
    (message.mid.clone(), name.to_owned())
}

/// The `call_key` and `schema_pin` a display entry of call `key` carries on
/// a runner declaring `call_identity`, with the fake's deliberate breaks.
/// `live` is set for a live tool event and clear for a display history row,
/// so a break can hit one surface and leave the other right, as a runner
/// whose two code paths disagree would.
fn call_identity(
    world: &World,
    session: &str,
    sess: &Sess,
    key: &str,
    live: bool,
) -> (Option<String>, Option<String>) {
    let defects = &world.defects;
    let Some(call) = sess.calls.get(key) else {
        return (None, None);
    };
    let shown_key = if defects.call_identity_omits_key
        || (live && defects.call_identity_live_omits_key)
    {
        None
    } else if defects.call_identity_swaps_keys || (live && defects.call_identity_live_swaps_keys) {
        let siblings = sess
            .messages
            .iter()
            .find(|m| m.calls.iter().any(|k| k == key))
            .map(|m| m.calls.clone())
            .unwrap_or_default();
        siblings
            .into_iter()
            .find(|k| k != key)
            .or_else(|| Some(key.to_owned()))
    } else {
        Some(key.to_owned())
    };
    let pin = if call.denied {
        (defects.call_identity_pin_on_denied || (live && defects.call_identity_live_pin_on_denied))
            .then(|| world.catalog_pin(session))
    } else if call.intent && defects.call_identity_pin_from_catalog {
        Some(world.catalog_pin(session))
    } else {
        call.schema_pin.clone()
    };
    (shown_key, pin)
}

/// The stored live event's member naming the call it is about, which the
/// fake removes before serving the event.
const EVENT_CALL_KEY: &str = "fake_call_key";

/// Record the typed live events the record just applied maps onto: the
/// run's start and end, a call's start and finish, and every message it
/// wrote, each started then finished.
pub(super) fn live_events(
    sess: &mut Sess,
    kind: &str,
    record: &Value,
    before: &BTreeSet<String>,
    defects: &Defects,
) {
    let index = sess.events.len() - 1;
    let at = record["at"].as_u64();
    let mut out: Vec<Value> = Vec::new();
    let event = |e: LiveEvent| serde_json::to_value(e).expect("events serialize");
    match kind {
        "send" if record["no_run"] != json!(true) => {
            if let Some(run_id) = record["run_id"].as_str().filter(|id| !id.is_empty()) {
                // A run a send started names that send; a host turn has none.
                let mut started = RunStarted::new(run_id);
                if let Some(cause) = record["cause"].as_str() {
                    started = started.with_send_id(cause);
                }
                if let Some(stamp) = super::rich_ui::run_stamp(defects, sess, run_id) {
                    started = started.with_rich_ui(stamp);
                }
                out.push(event(LiveEvent::RunStarted(started)));
            }
        }
        "intent" => {
            let key = record["call_key"].as_str().unwrap_or_default();
            if let Some(call) = sess.calls.get(key) {
                let (mid, name) = call_holder(sess, key);
                // The arguments are cut per subscription, so the whole value
                // is kept here beside the event.
                let mut started = event(LiveEvent::ToolStarted(ToolStarted::new(
                    mid,
                    &call.tool_call_id,
                    name,
                    &call.arguments,
                    None,
                    at,
                )));
                started["arguments_value"] = call.arguments.clone();
                started[EVENT_CALL_KEY] = json!(key);
                out.push(started);
            }
        }
        "result" if !defects.live_tool_finished_missing => {
            let key = record["call_key"].as_str().unwrap_or_default();
            if let Some(call) = sess.calls.get(key) {
                let (mid, _) = call_holder(sess, key);
                let outcome = if call.denied {
                    live::tool_outcomes::DENIED
                } else {
                    live::tool_outcomes::OK
                };
                let mut finished = event(LiveEvent::ToolFinished(ToolFinished::new(
                    mid,
                    &call.tool_call_id,
                    outcome,
                    result_text(&record["content"]).len() as u64,
                    at,
                )));
                finished[EVENT_CALL_KEY] = json!(key);
                out.push(finished);
            }
        }
        "legacy_call" => {
            // The stored call has no recorded times, so its events carry
            // none, unless the defect stamps the replay's own time on both.
            let invented = at.filter(|_| defects.live_tool_time_invented);
            let key = record["call_key"].as_str().unwrap_or_default();
            if let Some(call) = sess.calls.get(key) {
                let (mid, name) = call_holder(sess, key);
                let mut started = event(LiveEvent::ToolStarted(ToolStarted::new(
                    &mid,
                    &call.tool_call_id,
                    name,
                    &call.arguments,
                    None,
                    invented,
                )));
                started["arguments_value"] = call.arguments.clone();
                out.push(started);
                if !defects.live_tool_finished_missing {
                    out.push(event(LiveEvent::ToolFinished(ToolFinished::new(
                        mid,
                        &call.tool_call_id,
                        live::tool_outcomes::OK,
                        result_text(&record["content"]).len() as u64,
                        invented,
                    ))));
                }
            }
        }
        _ => {}
    }
    for message in sess.messages.iter().filter(|m| !before.contains(&m.mid)) {
        out.push(event(LiveEvent::MessageStarted(MessageEvent::new(
            &message.mid,
        ))));
        out.push(event(LiveEvent::MessageFinished(MessageEvent::new(
            &message.mid,
        ))));
    }
    if kind == "terminal" && !defects.live_run_finished_missing {
        out.push(event(LiveEvent::RunFinished(RunEnded::new(
            record["run_id"].as_str().unwrap_or_default(),
            record["state"].as_str().unwrap_or_default(),
        ))));
    }
    sess.live.extend(out.into_iter().map(|e| (index, e)));
}

/// The row of a body in the fake's own schema.
fn own_row(body: &Value, limits: Option<&Limits>) -> DisplayRow {
    let role = body["role"].as_str().unwrap_or("unknown");
    let cut = |text: String, bound: Option<u64>| match bound {
        Some(bound) => display_rows::cut(&text, bound),
        None => (text, false),
    };
    let mut parts: Vec<Value> = Vec::new();
    if let Some(text) = body["text"].as_str() {
        parts.push(json!({ "type": "text", "part": "0", "text": text }));
    }
    for (index, part) in body["parts"].as_array().into_iter().flatten().enumerate() {
        let key = index.to_string();
        match part["type"].as_str() {
            Some("tool_call") => {
                let (arguments, truncated) = cut(
                    part["arguments"].to_string(),
                    limits.map(|l| l.arguments_bytes),
                );
                parts.push(
                    json!({ "type": "tool_call", "part": key, "tool_call_id": part["id"],
                                   "name": part["tool"], "arguments": arguments,
                                   "arguments_truncated": truncated }),
                );
            }
            Some(kind @ ("text" | "reasoning")) => {
                parts.push(json!({ "type": kind, "part": key, "text": part["text"] }))
            }
            other => parts.push(json!({ "type": "other", "part": key, "kind": other })),
        }
    }
    if role == "tool" && !body["parts"].is_array() {
        let (content, truncated) = cut(
            result_text(&body["content"]),
            limits.map(|l| l.result_bytes),
        );
        let id = body["call_key"]
            .as_str()
            .or_else(|| body["tool_call_id"].as_str());
        parts.push(
            json!({ "type": "tool_result", "part": "0", "tool_call_id": id.unwrap_or_default(),
                           "content": content, "truncated": truncated }),
        );
    }
    let parts: Vec<DisplayPart> = parts
        .into_iter()
        .map(|part| serde_json::from_value(part).expect("display parts decode"))
        .collect();
    DisplayRow::new(role, parts)
}

/// Keep exactly `bound` bytes of `text`, splitting a character when the
/// bound falls inside one: the cut a display case must catch.
fn lossy(text: &str, bound: u64) -> String {
    let end = (bound as usize).min(text.len());
    String::from_utf8_lossy(&text.as_bytes()[..end]).into_owned()
}

impl Module {
    /// The display row of `entry`, a raw history entry of `session`.
    fn display_row(
        &self,
        session: &str,
        sess: &Sess,
        entry: &HistoryEntry,
        limits: Option<&Limits>,
    ) -> DisplayRow {
        let defects = &self.world.defects;
        let limits = match limits {
            None if defects.display_default_limits => Some(&APP_LIMITS),
            limits => limits,
        };
        let project = |limits: Option<&Limits>| {
            if profiled(session) {
                display_rows::anthropic_messages(&entry.message, limits)
                    .unwrap_or_else(|_| DisplayRow::new("unknown", Vec::new()))
            } else {
                own_row(&entry.message, limits)
            }
        };
        let mut row = project(limits);
        if let Some(bounds) =
            limits.filter(|_| defects.display_cut_lossy || defects.display_cut_unflagged)
        {
            let whole = project(None);
            for (part, full) in row.parts.iter_mut().zip(&whole.parts) {
                match (part, full) {
                    (DisplayPart::ToolCall(call), DisplayPart::ToolCall(full))
                        if call.arguments_truncated =>
                    {
                        if defects.display_cut_lossy {
                            call.arguments = lossy(&full.arguments, bounds.arguments_bytes);
                        }
                        call.arguments_truncated = !defects.display_cut_unflagged;
                    }
                    (DisplayPart::ToolResult(result), DisplayPart::ToolResult(full))
                        if result.truncated =>
                    {
                        let bound = if result.is_error {
                            bounds.error_result_bytes
                        } else {
                            bounds.result_bytes
                        };
                        if defects.display_cut_lossy {
                            result.content = lossy(&full.content, bound);
                        }
                        result.truncated = !defects.display_cut_unflagged;
                    }
                    _ => {}
                }
            }
        }
        if row.role == "assistant"
            && !defects.display_drops_model
            && sess.messages.iter().any(|m| m.mid == entry.mid)
        {
            row = row.with_model(self.session_model(session));
        }
        if let Some(message) = sess.messages.iter().find(|m| m.mid == entry.mid) {
            if let Some(id) = &message.client_message_id {
                row = row.with_client_message_id(id);
            }
            if let Some(sender) = message.sender.as_ref().filter(|_| row.role == "user") {
                row = row.with_sender(sender);
            }
        }
        output_refs::decorate(sess, &entry.mid, &mut row);
        self.stamp_row(sess, &entry.mid, &mut row);
        if self.world.serves(groups::CALL_IDENTITY) {
            if let Some(message) = sess.messages.iter().find(|m| m.mid == entry.mid) {
                for call in row.tool_calls_mut() {
                    let key = message.calls.iter().find(|key| {
                        sess.calls
                            .get(*key)
                            .is_some_and(|c| c.tool_call_id == call.tool_call_id)
                    });
                    if let Some(key) = key {
                        let (call_key, pin) = call_identity(&self.world, session, sess, key, false);
                        call.call_key = call_key;
                        call.schema_pin = pin;
                    }
                }
            }
        }
        row
    }

    /// `page` in display form.
    pub(super) fn display_reply(
        &self,
        session: &str,
        sess: &Sess,
        page: &HistoryPage,
        limits: Option<&Limits>,
    ) -> Reply {
        let mut shown = DisplayPage::from_history(page, |entry| {
            Ok::<_, ()>(self.display_row(session, sess, entry, limits))
        })
        .expect("rows never fail");
        if self.world.defects.display_in_progress_row
            && self.world.held_models.lock().unwrap().contains_key(session)
        {
            let row =
                DisplayRow::new("assistant", Vec::new()).with_model(self.session_model(session));
            shown.messages.push(
                DisplayEntry::new(
                    format!("{}.in-progress", sess.lineage),
                    self.world.now(),
                    row,
                )
                .with_ordinal(sess.base + sess.messages.len() as u64),
            );
        }
        respond(shown)
    }

    /// `session.subscribe` on a runner declaring `live_events`: every
    /// durable record from the attach point, one control frame per typed
    /// event it maps onto, then the attach replay of the step in flight,
    /// every frame numbered by `seq`.
    pub(super) fn live_subscribe(&self, session: &str, params: Value) -> SubscribeOutcome {
        let request: LiveSubscribeRequest = match serde_json::from_value(params) {
            Ok(request) => request,
            Err(e) => {
                let field = if e.to_string().contains("limits") {
                    "limits"
                } else {
                    "from"
                };
                let Reply::Error(error) = invalid(field) else {
                    unreachable!()
                };
                return SubscribeOutcome::Refused(error);
            }
        };
        let sessions = self.sessions.lock().unwrap();
        let (events, recorded) = sessions
            .get(session)
            .map(|s| (s.events.clone(), s.live.clone()))
            .unwrap_or_default();
        let after = match request.attach_point() {
            SubscribeFrom::Start => (0, u64::MAX),
            SubscribeFrom::Live => (events.len() as u64, u64::MAX),
            SubscribeFrom::After(position) => (
                position.get("seq").and_then(Value::as_u64).unwrap_or(0),
                position
                    .get("sub")
                    .and_then(Value::as_u64)
                    .unwrap_or(u64::MAX),
            ),
        };
        let mut frames: Vec<SubscribeEvent> = Vec::new();
        for (index, kind) in events.iter().enumerate() {
            let seq = index as u64 + 1;
            let mapped: Vec<&Value> = recorded
                .iter()
                .filter(|(at, _)| *at == index)
                .map(|(_, event)| event)
                .collect();
            let mut unit = Map::new();
            unit.insert("record".into(), json!(kind));
            if mapped.is_empty() {
                if (seq, u64::MAX) > after {
                    let mut frame = SubscribeEvent::new(kinds::CONTROL);
                    frame.cursor = Some(cursor(index + 1));
                    frame.body = unit;
                    frames.push(frame);
                }
                continue;
            }
            for (sub, event) in mapped.into_iter().enumerate() {
                if (seq, sub as u64) <= after {
                    continue;
                }
                let mut event = event.clone();
                let call_key = event
                    .as_object_mut()
                    .and_then(|e| e.remove(EVENT_CALL_KEY))
                    .and_then(|key| key.as_str().map(str::to_owned));
                if let Some(arguments) = event
                    .as_object_mut()
                    .and_then(|e| e.remove("arguments_value"))
                {
                    let started = ToolStarted::new(
                        event["mid"].as_str().unwrap_or_default(),
                        event["tool_call_id"].as_str().unwrap_or_default(),
                        event["name"].as_str().unwrap_or_default(),
                        &arguments,
                        request.limits.as_ref(),
                        event["started_at"].as_u64(),
                    );
                    event = serde_json::to_value(LiveEvent::ToolStarted(started)).unwrap();
                }
                if let (Some(key), Some(sess), true) = (
                    call_key,
                    sessions.get(session),
                    self.world.serves(groups::CALL_IDENTITY),
                ) {
                    let (call_key, pin) = call_identity(&self.world, session, sess, &key, true);
                    if let Some(call_key) = call_key {
                        event["call_key"] = json!(call_key);
                    }
                    if let Some(pin) = pin {
                        event["schema_pin"] = json!(pin);
                    }
                }
                let mut position = cursor(index + 1);
                position.insert("sub".into(), json!(sub));
                let mut frame = SubscribeEvent::new(kinds::CONTROL);
                frame.cursor = Some(position);
                frame.body = unit.clone();
                frame.body.insert(live::EVENT_FIELD.into(), event);
                frames.push(frame);
            }
        }
        drop(sessions);
        let in_flight = self.world.in_flight.lock().unwrap().get(session).cloned();
        if let Some((mid, parts)) =
            in_flight.filter(|_| !self.world.defects.live_attach_skips_replay)
        {
            let display = |event: LiveEvent| SubscribeEvent::new(kinds::DISPLAY).with_event(&event);
            frames.push(display(LiveEvent::DisplayReset));
            for (index, part) in parts.iter().enumerate() {
                let key = index.to_string();
                frames.push(display(match part {
                    ScriptedPart::Text(text) => {
                        LiveEvent::TextDelta(TextDelta::new("", text, &mid, key))
                    }
                    ScriptedPart::Reasoning(text) => {
                        LiveEvent::ReasoningDelta(TextDelta::new("", text, &mid, key))
                    }
                    ScriptedPart::ToolCall(call) => LiveEvent::ToolInputDelta(ToolInputDelta::new(
                        &call.tool_call_id,
                        call.arguments.to_string(),
                        &mid,
                        key,
                    )),
                }));
            }
        }
        let mut next = (live::FIRST_SEQ, live::FIRST_SEQ);
        SubscribeOutcome::Events(
            frames
                .into_iter()
                .map(|frame| {
                    let display =
                        frame.kind == kinds::DISPLAY && self.world.defects.live_seq_per_lane;
                    let counter = if display { &mut next.1 } else { &mut next.0 };
                    let seq = *counter;
                    *counter += 1;
                    if !self.world.defects.live_seq_per_lane {
                        next.1 = next.0;
                    }
                    serde_json::to_value(frame.with_seq(seq)).expect("frames serialize")
                })
                .collect(),
        )
    }
}
