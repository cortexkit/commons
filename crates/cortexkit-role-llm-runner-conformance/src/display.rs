//! The display cases: `session.history` with `form: "display"` (the
//! `history_display` group), `client_message_id` on `session.send` (the
//! `client_message_id` group) and the `sender` a row names,
//! `cortexkit-role-llm-runner` CONTRACT §4.7 and §9.11.
//!
//! Sessions here are minted with the label `display`, so an adapter that
//! keeps some sessions in a serializer profile can tell them apart.

use cortexkit_role_llm_runner::{
    delivery_order::{DeliveryOrder, Mode},
    display::{self, DisplayEntry, DisplayPage, DisplayPart, Limits, APP_LIMITS},
    errors,
    history::{HistoryEntry, HistoryForm, HistoryRequest},
    ops,
    send::{SendReply, CLIENT_MESSAGE_ID_FIELD},
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_error, expect_field, expect_ok, head, send_params, wait_run_end},
    host_history::walk_older,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn},
    RunnerRoute,
};

type Check = Result<(), String>;

/// Entries per display page, so every walk follows cursors.
const PAGE: u64 = 2;

/// How many pages a walk follows before it gives up.
const MAX_PAGES: usize = 1_000;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "display_row_is_the_profile_projection" => projection(case).await,
        "display_own_run_records_model" => own_run_model(case).await,
        "display_limits_cut_on_a_utf8_boundary" => limits(case).await.map(|()| Ending::Passed),
        "display_without_limits_is_whole" => whole(case).await.map(|()| Ending::Passed),
        "display_pages_carry_finished_messages_only" => finished_only(case).await,
        "client_message_id_on_display_rows_never_in_read" => {
            client_id_shown(case).await.map(|()| Ending::Passed)
        }
        "client_message_id_resend_with_another_id_refused" => {
            client_id_reuse(case).await.map(|()| Ending::Passed)
        }
        "display_row_names_the_sender_never_in_read" => {
            sender_shown(case).await.map(|()| Ending::Passed)
        }
        other => Err(format!("unknown display case {other}")),
    }
}

/// One display page, decoded and checked on its own.
async fn display_page<R: RunnerRoute>(
    route: &R,
    request: &HistoryRequest,
) -> Result<DisplayPage, String> {
    let body = expect_ok(
        "a display session.history",
        call(
            route,
            ops::SESSION_HISTORY,
            serde_json::to_value(request).expect("a history request serializes"),
        )
        .await?,
    )?;
    let page: DisplayPage = decode("a display session.history", body)?;
    page.check(head(route).await?.first_ordinal)
        .map_err(|problem| format!("a display page breaks a history page's rules: {problem:?}"))?;
    Ok(page)
}

/// Every display entry, oldest first, from the newest page back through
/// `older`.
pub(crate) async fn display_walk<R: RunnerRoute>(
    route: &R,
    limits: Option<Limits>,
) -> Result<Vec<DisplayEntry>, String> {
    let request = |base: HistoryRequest| {
        let request = base.with_form(HistoryForm::Display).with_limit(PAGE);
        match limits {
            Some(limits) => request.with_limits(limits),
            None => request,
        }
    };
    let mut page = display_page(route, &request(HistoryRequest::newest())).await?;
    let mut pages = vec![page.messages.clone()];
    for _ in 0..MAX_PAGES {
        let Some(older) = page.older.clone() else {
            return Ok(pages.into_iter().rev().flatten().collect());
        };
        page = display_page(route, &request(HistoryRequest::cursor(older))).await?;
        if page.messages.is_empty() {
            return Err("an older cursor answered an empty display page".into());
        }
        pages.push(page.messages.clone());
    }
    Err(format!(
        "display history did not end within {MAX_PAGES} pages"
    ))
}

fn mids(entries: &[DisplayEntry]) -> Vec<&str> {
    entries.iter().map(|e| e.mid.as_str()).collect()
}

/// The display walk serves exactly the raw walk's entries, in order.
fn same_entries(what: &str, raw: &[HistoryEntry], shown: &[DisplayEntry]) -> Check {
    let raw_mids: Vec<&str> = raw.iter().map(|e| e.mid.as_str()).collect();
    if raw_mids != mids(shown) {
        return Err(format!(
            "{what}, the display form served {:?}, but the raw form serves {raw_mids:?}: the \
             display form serves the same entries",
            mids(shown)
        ));
    }
    for (raw, shown) in raw.iter().zip(shown) {
        if (raw.ordinal, raw.written_at, &raw.tags)
            != (shown.ordinal, shown.written_at, &shown.tags)
        {
            return Err(format!(
                "{what}, entry {} carries ordinal {:?}, written_at {} and tags {:?} in display \
                 form, but {:?}, {} and {:?} raw: the envelope is kept",
                raw.mid,
                shown.ordinal,
                shown.written_at,
                shown.tags,
                raw.ordinal,
                raw.written_at,
                raw.tags
            ));
        }
    }
    Ok(())
}

/// A display session: a prompt, an answer of reasoning, text and a tool
/// call with `call`'s arguments and result, and a closing text.
async fn written<S>(case: &Case<'_, S>, call: ScriptedToolCall) -> Result<Session<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let script = Script {
        turns: vec![
            ScriptedTurn::new(vec![
                ScriptedPart::Reasoning(case.mint.next("display-thinking")),
                case.mint.text("display-text").0,
                ScriptedPart::ToolCall(call),
            ]),
            ScriptedTurn::new(vec![case.mint.text("display-closing").0]),
        ],
    };
    let mut session = case.open("display", script).await?;
    case.run_prompt(&mut session, &case.mint.next("display-prompt"))
        .await?;
    Ok(session)
}

/// A served row equals the profile's projection of the same entry's raw
/// body, with only the fields the runner fills (`model`,
/// `client_message_id` and `sender`) taken from the served row. Imported rows
/// may lack a recorded model. Checked without limits and with the apps' limits.
async fn projection<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = written(case, case.mint.call("display-call")).await?;
    let Some(profile) = case.subject.serializer_profile(&session.name) else {
        return Ok(Ending::Inapplicable(
            "the adapter names no serializer profile for the session's bodies, so the suite \
             cannot project them"
                .into(),
        ));
    };
    if !display::projects(&profile) {
        return Ok(Ending::Inapplicable(format!(
            "{profile} has no shared display projection in llm-runner, so this check cannot \
             compare its rows with one"
        )));
    }
    let raw = walk_older(&session.route).await?;
    for limits in [None, Some(APP_LIMITS)] {
        let what = match limits {
            None => "without limits",
            Some(_) => "with the apps' limits",
        };
        let shown = display_walk(&session.route, limits).await?;
        same_entries(what, &raw, &shown)?;
        for (raw, shown) in raw.iter().zip(&shown) {
            let mut expected =
                display::project(&profile, &raw.message, limits.as_ref()).map_err(|e| {
                    format!(
                        "the raw body of {} does not project as {profile}: {e}",
                        raw.mid
                    )
                })?;
            if let Some(model) = &shown.row.model {
                expected = expected.with_model(model.clone());
            }
            expected.client_message_id = shown.row.client_message_id.clone();
            expected.sender = shown.row.sender.clone();
            // Under call_identity the runner adds a tool call's key and, for a
            // dispatched provider tool, its pin. The stored body carries
            // neither, so they're copied here and the call_identity cases
            // check their values.
            for (call, served) in expected
                .tool_calls_mut()
                .zip(shown.row.parts.iter().filter_map(|part| match part {
                    DisplayPart::ToolCall(call) => Some(call),
                    _ => None,
                }))
            {
                call.call_key = served.call_key.clone();
                call.schema_pin = served.schema_pin.clone();
            }
            if shown.row != expected {
                return Err(format!(
                    "{what}, the display row of {} is {:?}, but {profile}'s projection of its raw \
                     body is {expected:?}",
                    raw.mid, shown.row
                ));
            }
        }
    }
    Ok(Ending::Passed)
}

/// The unique scripted answer identifies a message the kit drove through
/// the runner itself, rather than an imported assistant message.
async fn own_run_model<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let marker = case.mint.next("recorded-model");
    let mut session = case
        .open(
            "display-model",
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(marker.clone())])],
            },
        )
        .await?;
    case.run_prompt(&mut session, &case.mint.next("model-prompt"))
        .await?;
    let rows = display_walk(&session.route, None).await?;
    let own: Vec<_> = rows
        .iter()
        .filter(|e| e.row.role == "assistant" && e.row.text == marker)
        .collect();
    if own.is_empty() {
        return Err("display omitted the kit's scripted assistant message".into());
    }
    if own.iter().any(|e| e.row.model.is_none()) {
        return Err("a runner-written assistant row carries no model".into());
    }
    Ok(Ending::Passed)
}

/// The byte offset inside the first multi-byte character of `text`, if any.
fn inside_a_character(text: &str) -> Option<u64> {
    (1..text.len())
        .find(|i| !text.is_char_boundary(*i))
        .map(|i| i as u64)
}

fn tool_parts<'a>(
    entries: &'a [DisplayEntry],
    tool_call_id: &str,
) -> (Option<&'a str>, Option<&'a str>) {
    let mut arguments = None;
    let mut content = None;
    for part in entries.iter().flat_map(|e| &e.row.parts) {
        match part {
            DisplayPart::ToolCall(call) if call.tool_call_id == tool_call_id => {
                arguments = Some(call.arguments.as_str())
            }
            DisplayPart::ToolResult(result) if result.tool_call_id == tool_call_id => {
                content = Some(result.content.as_str())
            }
            _ => {}
        }
    }
    (arguments, content)
}

/// With limits that fall inside a multi-byte character of the call's
/// arguments and of its result, each is cut back to the character boundary
/// at or below its bound and flagged; everything else is served whole and
/// unflagged.
async fn limits<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut call = case.mint.call("display-cut");
    call.arguments["text"] = json!("ééé漢字🦀🦀");
    call.result["text"] = json!("漢字漢字🦀🦀");
    let session = written(case, call.clone()).await?;
    let whole = display_walk(&session.route, None).await?;
    let (Some(arguments), Some(content)) = tool_parts(&whole, &call.tool_call_id) else {
        return Err(format!(
            "no display row shows tool call {} with its result",
            call.tool_call_id
        ));
    };
    let (Some(arguments_bytes), Some(result_bytes)) =
        (inside_a_character(arguments), inside_a_character(content))
    else {
        return Err(format!(
            "the call's arguments {arguments:?} or its result {content:?} hold no multi-byte \
             character, though the scripted ones do"
        ));
    };
    let bounds = Limits::new(arguments_bytes, result_bytes, u64::MAX);
    let cut = display_walk(&session.route, Some(bounds)).await?;
    if mids(&whole) != mids(&cut) {
        return Err("a display read with limits served other entries than one without".into());
    }
    let mut flagged = (false, false);
    for (whole, cut) in whole.iter().zip(&cut) {
        if whole.row.parts.len() != cut.row.parts.len() {
            return Err(format!("limits changed the parts of {}", whole.mid));
        }
        for (full, shown) in whole.row.parts.iter().zip(&cut.row.parts) {
            let expected = match full {
                DisplayPart::ToolCall(call) => {
                    let (text, truncated) = display::cut(&call.arguments, bounds.arguments_bytes);
                    flagged.0 |= truncated;
                    let mut call = call.clone();
                    call.arguments = text;
                    call.arguments_truncated = truncated;
                    DisplayPart::ToolCall(call)
                }
                DisplayPart::ToolResult(result) => {
                    let bound = if result.is_error {
                        bounds.error_result_bytes
                    } else {
                        bounds.result_bytes
                    };
                    let (text, truncated) = display::cut(&result.content, bound);
                    flagged.1 |= truncated;
                    let mut result = result.clone();
                    result.content = text;
                    result.truncated = truncated;
                    DisplayPart::ToolResult(result)
                }
                other => other.clone(),
            };
            if *shown != expected {
                return Err(format!(
                    "with limits {bounds:?}, {} serves {shown:?}, but the same part read whole is \
                     {full:?}: a cut ends at the last UTF-8 character boundary at or below its \
                     bound and sets arguments_truncated or truncated, and nothing else changes",
                    whole.mid
                ));
            }
        }
    }
    if flagged != (true, true) {
        return Err("the bounds cut nothing, though they fall inside the content".into());
    }
    Ok(())
}

/// Without limits, a call's arguments and result larger than every bound of
/// the apps' limits are served whole and unflagged; with those limits, both
/// are cut.
async fn whole<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut call = case.mint.call("display-whole");
    let long_arguments = "a".repeat(APP_LIMITS.arguments_bytes as usize * 2);
    let long_result = "r".repeat(APP_LIMITS.error_result_bytes as usize * 2);
    call.arguments["text"] = json!(long_arguments);
    call.result["text"] = json!(long_result);
    let session = written(case, call.clone()).await?;
    let shown = display_walk(&session.route, None).await?;
    for part in shown.iter().flat_map(|e| &e.row.parts) {
        let truncated = match part {
            DisplayPart::ToolCall(call) => call.arguments_truncated,
            DisplayPart::ToolResult(result) => result.truncated,
            _ => false,
        };
        if truncated {
            return Err(format!(
                "a display read without limits served {part:?} cut: absent limits mean whole \
                 content, and a runner never cuts by a default of its own"
            ));
        }
    }
    let (Some(arguments), Some(content)) = tool_parts(&shown, &call.tool_call_id) else {
        return Err(format!(
            "no display row shows tool call {} with its result",
            call.tool_call_id
        ));
    };
    let parsed: Value = serde_json::from_str(arguments)
        .map_err(|e| format!("the call's arguments {arguments:?} are not JSON text: {e}"))?;
    if parsed != call.arguments || !content.contains(&long_result) {
        return Err(format!(
            "a display read without limits served arguments of {} bytes and a result of {} \
             bytes, not the whole call: absent limits mean whole content",
            arguments.len(),
            content.len()
        ));
    }
    let cut = display_walk(&session.route, Some(APP_LIMITS)).await?;
    let (Some(arguments), Some(content)) = tool_parts(&cut, &call.tool_call_id) else {
        return Err("the cut read lost the tool call".into());
    };
    if arguments.len() as u64 > APP_LIMITS.arguments_bytes
        || content.len() as u64 > APP_LIMITS.result_bytes
    {
        return Err(format!(
            "with the apps' limits the call's arguments are {} bytes and its result {} bytes, \
             over their bounds",
            arguments.len(),
            content.len()
        ));
    }
    Ok(())
}

/// While a model step is held, the display page serves exactly the raw
/// page's finished entries, never an entry for the step in flight; once the
/// run ends, the answer is there.
async fn finished_only<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (held_answer, marker) = case.mint.text("display-held-answer");
    let mut session = case
        .open(
            "display",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("display-first-answer").0]),
                    ScriptedTurn::new(vec![held_answer]),
                ],
            },
        )
        .await?;
    case.run_prompt(&mut session, &case.mint.next("display-first"))
        .await?;
    if let Err(e) = case.subject.hold_next_model_step(&session.name).await {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot hold a model step, so no step is in flight to read past: {e}"
        )));
    }
    let before = head(&session.route).await?.last_run_state.map(|l| l.run_id);
    let send = send_params(
        case.subject,
        &session.name,
        false,
        &case.mint.next("display-second"),
        &case.mint.next("send"),
        None,
    );
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, send).await?,
        )?,
    )?;
    case.subject
        .await_model_step(&session.name)
        .await
        .map_err(|e| format!("waiting for the held model step: {e}"))?;
    let raw = walk_older(&session.route).await?;
    let shown = display_walk(&session.route, None).await?;
    let held = same_entries("while a model step is in flight", &raw, &shown);
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| format!("releasing the held model step: {e}"))?;
    held.map_err(|e| format!("{e}; a display page carries finished messages only"))?;
    if let Some(entry) = shown.iter().find(|e| e.row.text.contains(&marker)) {
        return Err(format!(
            "while its step was held, the display page served {} with the held answer",
            entry.mid
        ));
    }
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    let after = display_walk(&session.route, None).await?;
    if !after.iter().any(|e| e.row.text.contains(&marker)) {
        return Err("once the run ended, no display row shows the held answer".into());
    }
    Ok(Ending::Passed)
}

/// Every member of `value`, at any depth, and every string it holds.
fn mentions(value: &Value, key: &str, text: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(k, v)| k == key || mentions(v, key, text)),
        Value::Array(items) => items.iter().any(|v| mentions(v, key, text)),
        Value::String(s) => s.contains(text),
        _ => false,
    }
}

/// The user message a send with `client_message_id` created shows the id on
/// its display row, no other row shows it, and the default `session.read`
/// entry carries neither the member nor the id anywhere.
async fn client_id_shown<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case
        .open(
            "display",
            Script {
                turns: vec![ScriptedTurn::new(vec![case.mint.text("display-answer").0])],
            },
        )
        .await?;
    let id = case.mint.next("cmid");
    let prompt = case.mint.next("display-prompt");
    let mut send = send_params(
        case.subject,
        &session.name,
        true,
        &prompt,
        &case.mint.next("send"),
        None,
    );
    send[CLIENT_MESSAGE_ID_FIELD] = json!(id);
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "a send with a client_message_id",
            call(&session.route, ops::SESSION_SEND, send).await?,
        )?,
    )?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let shown = display_walk(&session.route, None).await?;
    let carrying: Vec<&DisplayEntry> = shown
        .iter()
        .filter(|e| e.row.client_message_id.is_some())
        .collect();
    match carrying.as_slice() {
        [entry]
            if entry.row.client_message_id.as_deref() == Some(id.as_str())
                && entry.row.text.contains(&prompt) => {}
        _ => {
            return Err(format!(
                "the display rows carrying a client_message_id are {:?}: exactly the user message \
                 the send created carries {id}",
                carrying
                    .iter()
                    .map(|e| (&e.mid, &e.row.client_message_id))
                    .collect::<Vec<_>>()
            ))
        }
    }
    let page = expect_ok(
        "session.read",
        call(&session.route, ops::SESSION_READ, json!({})).await?,
    )?;
    if mentions(&page, CLIENT_MESSAGE_ID_FIELD, &id) {
        return Err(format!(
            "the default session.read page {page} carries client_message_id {id}: it is shown \
             only on display rows, never in a session.read entry"
        ));
    }
    Ok(())
}

/// A re-send under the same `send_id` with the same `client_message_id` is
/// a retry; with another one, or without one, it is refused
/// `send_id_reuse` naming `client_message_id`; an id outside the grammar is
/// refused `invalid_params` naming it.
async fn client_id_reuse<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case
        .open(
            "display",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("display-answer").0]),
                    ScriptedTurn::new(vec![case.mint.text("display-answer").0]),
                ],
            },
        )
        .await?;
    let id = case.mint.next("cmid");
    let mut send = send_params(
        case.subject,
        &session.name,
        true,
        &case.mint.next("display-prompt"),
        &case.mint.next("send"),
        None,
    );
    send[CLIENT_MESSAGE_ID_FIELD] = json!(id);
    let route = &session.route;
    let first: SendReply = decode(
        "session.send",
        expect_ok(
            "a send with a client_message_id",
            call(route, ops::SESSION_SEND, send.clone()).await?,
        )?,
    )?;
    wait_run_end(case.subject, route, case.declared, None, &first).await?;
    expect_ok(
        "a re-send with the same client_message_id",
        call(route, ops::SESSION_SEND, send.clone()).await?,
    )?;
    let mut other = send.clone();
    other[CLIENT_MESSAGE_ID_FIELD] = json!(case.mint.next("cmid"));
    let mut without = send.clone();
    without
        .as_object_mut()
        .expect("send params are an object")
        .remove(CLIENT_MESSAGE_ID_FIELD);
    for (what, params) in [
        (
            "a re-send under the same send_id with another client_message_id",
            other,
        ),
        (
            "a re-send under the same send_id without its client_message_id",
            without,
        ),
    ] {
        let error = expect_error(what, call(route, ops::SESSION_SEND, params).await?)?;
        expect_field(what, &error, errors::SEND_ID_REUSE, CLIENT_MESSAGE_ID_FIELD)?;
    }
    let mut bad = send_params(
        case.subject,
        &session.name,
        false,
        &case.mint.next("display-prompt"),
        &case.mint.next("send"),
        None,
    );
    bad[CLIENT_MESSAGE_ID_FIELD] = json!("not a valid id");
    let what = "a send whose client_message_id is outside the grammar";
    let error = expect_error(what, call(route, ops::SESSION_SEND, bad).await?)?;
    expect_field(
        what,
        &error,
        errors::INVALID_PARAMS,
        CLIENT_MESSAGE_ID_FIELD,
    )
}

/// Whether `value` holds a member named `key` at any depth.
fn has_member(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(k, v)| k == key || has_member(v, key)),
        Value::Array(items) => items.iter().any(|v| has_member(v, key)),
        _ => false,
    }
}

/// A plan sender's `[idle_start]` message has a display row naming the
/// sender's module as `sender`, and the default `session.read` page carries
/// no `sender` member anywhere.
async fn sender_shown<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let identity = case
        .subject
        .plan_sender()
        .ok_or("the subject names no plan sender, which this case requires")?;
    let (session, named) = crate::delivery_order::sender_session(
        case,
        "display-sender",
        vec![ScriptedTurn::new(vec![case.mint.text("display-answer").0])],
        true,
        false,
    )
    .await?;
    let before = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id);
    let marker = case.mint.next("display-sender");
    let order = DeliveryOrder::new(vec![Mode::IdleStart]);
    let what = "the plan sender's [idle_start] send";
    let reply: SendReply = decode(
        what,
        expect_ok(
            what,
            call(
                &named,
                ops::SESSION_SEND,
                crate::tags::params(
                    case,
                    &session,
                    None,
                    &marker,
                    &case.mint.next("send"),
                    &order.to_send_members(),
                ),
            )
            .await?,
        )?,
    )?;
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    let shown = display_walk(&session.route, None).await?;
    let Some(entry) = shown
        .iter()
        .find(|e| e.row.role == "user" && e.row.text.contains(&marker))
    else {
        return Err(format!("no user display row shows the message of {what}"));
    };
    if entry.row.sender.as_deref() != Some(identity.module.as_str()) {
        return Err(format!(
            "the display row of {what} names sender {:?}, not {}: a user message's row names the \
             module that sent it",
            entry.row.sender, identity.module
        ));
    }
    let page = expect_ok(
        "session.read",
        call(&session.route, ops::SESSION_READ, json!({})).await?,
    )?;
    if has_member(&page, "sender") {
        return Err(format!(
            "the default session.read page {page} carries a sender member: who sent a message is \
             shown only on display rows, never in a session.read entry or a stored body"
        ));
    }
    Ok(())
}
