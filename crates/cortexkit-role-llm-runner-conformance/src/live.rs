//! The live-event cases: typed events on `session.subscribe` (the
//! `live_events` group, `cortexkit-role-llm-runner` CONTRACT §7.1), and the
//! `run_cause` cases, which read the `send_id` of `run_started`.
//!
//! The route collects a stream only until it goes quiet
//! ([`RunnerRoute::subscribe`]), so the suite reads replays of idle sessions
//! and, for the attach replay, a stream attached while the adapter holds a
//! model answer mid-stream.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    live::{LiveEvent, LiveSubscribeRequest, FIRST_SEQ},
    ops,
    send::SendReply,
    subscribe::{kinds, SubscribeEvent, SubscribeFrom},
};
use serde_json::{Map, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_ok, head, read_all, send_params, wait_run_end, MAX_POLLS},
    route::SubscribeOutcome,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute, SCRIPTED_TOOL,
};

type Check = Result<(), String>;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "live_message_and_tool_events_durable" => lifecycle(case).await.map(|()| Ending::Passed),
        "live_run_events" => runs(case).await.map(|()| Ending::Passed),
        "live_attach_replays_the_current_step" => attach(case).await,
        "live_seq_rises_across_lanes" => seq(case).await,
        "live_untimed_history_call_leaves_times_out" => untimed(case).await,
        "run_cause_queued_start_names_its_send" => {
            queued_cause(case).await.map(|()| Ending::Passed)
        }
        "run_cause_send_started_runs_never_omit_send_id" => {
            every_cause(case).await.map(|()| Ending::Passed)
        }
        "run_cause_interrupt_names_the_new_send" => interrupt_cause(case).await,
        other => Err(format!("unknown live-event case {other}")),
    }
}

/// One frame of a stream: the frame as decoded, and its event decoded
/// strictly when it carries one.
struct Frame {
    frame: SubscribeEvent,
    event: Option<LiveEvent>,
}

/// Every frame a subscription from `from` streams, each event decoded
/// strictly.
async fn stream<R: RunnerRoute>(route: &R, from: SubscribeFrom) -> Result<Vec<Frame>, String> {
    let params = serde_json::to_value(LiveSubscribeRequest::new(from)).expect("serializes");
    let events = match route
        .subscribe(params)
        .await
        .map_err(|e| format!("session.subscribe: the route failed: {}", e.message))?
    {
        SubscribeOutcome::Events(events) => events,
        SubscribeOutcome::Refused(error) => {
            return Err(format!(
                "session.subscribe was refused {} {:?}: {}",
                error.code, error.detail, error.message
            ))
        }
    };
    events
        .into_iter()
        .map(|raw| {
            let frame: SubscribeEvent = decode("a stream frame", raw)?;
            let event = frame
                .body
                .get("event")
                .map(|event| {
                    LiveEvent::decode_strict(event).map_err(|e| {
                        format!("a {} frame's event {event} is malformed: {e}", frame.kind)
                    })
                })
                .transpose()?;
            Ok(Frame { frame, event })
        })
        .collect()
}

/// A control frame's cursor and its event.
type ControlFrame = (Option<Map<String, Value>>, Option<LiveEvent>);

/// The `mid` an attach replay names, and each part's key and text.
type Replay = (String, Vec<(String, String)>);

/// The control frames' cursors and events, in order.
fn control(frames: &[Frame]) -> Vec<ControlFrame> {
    frames
        .iter()
        .filter(|f| f.frame.kind == kinds::CONTROL)
        .map(|f| (f.frame.cursor.clone(), f.event.clone()))
        .collect()
}

/// Every frame carries `seq`, the first [`FIRST_SEQ`], each one above the
/// one before, across both lanes.
fn check_seq(what: &str, frames: &[Frame]) -> Check {
    let seqs: Vec<Option<u64>> = frames.iter().map(|f| f.frame.seq()).collect();
    let expected: Vec<Option<u64>> = (0..frames.len() as u64)
        .map(|i| Some(FIRST_SEQ + i))
        .collect();
    if seqs != expected {
        return Err(format!(
            "{what}, the frames carry seq {seqs:?}: every frame of one stream carries seq, from \
             {FIRST_SEQ}, rising by one per frame across control and display frames"
        ));
    }
    Ok(())
}

/// A session that ran one prompt answered by a tool call and a closing
/// text. Returns it, the call and the run's id.
async fn with_tool_call<S>(
    case: &Case<'_, S>,
) -> Result<(Session<S::Route>, crate::subject::ScriptedToolCall, String), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let call = case.mint.call("live-call");
    let mut session = case
        .open(
            "live",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(call.clone())]),
                    ScriptedTurn::new(vec![case.mint.text("live-closing").0]),
                ],
            },
        )
        .await?;
    let (_, end) = case
        .run_prompt(&mut session, &case.mint.next("live-prompt"))
        .await?;
    Ok((session, call, end.run_id))
}

/// Every stored message has a `message_started` and, after it, a
/// `message_finished` naming its `mid`; the tool call has `tool_started`
/// (its message's `mid`, the model's id, the tool's name, the whole
/// arguments) and, after it, `tool_finished` with outcome `ok`. The events
/// are durable: a second replay serves the same control frames, and a
/// subscription after the `tool_started` frame serves exactly what followed
/// it.
async fn lifecycle<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, tool, _) = with_tool_call(case).await?;
    let route = &session.route;
    let frames = stream(route, SubscribeFrom::Start).await?;
    let replay = control(&frames);
    let events: Vec<(usize, &LiveEvent)> = replay
        .iter()
        .enumerate()
        .filter_map(|(i, (_, e))| e.as_ref().map(|e| (i, e)))
        .collect();
    let position = |wanted: &dyn Fn(&LiveEvent) -> bool| {
        events.iter().find(|(_, e)| wanted(e)).map(|(i, _)| *i)
    };
    let (_, messages) = read_all(route).await?;
    for message in &messages {
        let mid = message.mid.as_str();
        let started = position(&|e| matches!(e, LiveEvent::MessageStarted(m) if m.mid == mid));
        let finished = position(&|e| matches!(e, LiveEvent::MessageFinished(m) if m.mid == mid));
        match (started, finished) {
            (Some(started), Some(finished)) if started < finished => {}
            _ => {
                return Err(format!(
                    "message {mid} has message_started at {started:?} and message_finished at \
                     {finished:?} among the replayed control events: every stored message has \
                     both, durably, started first"
                ))
            }
        }
    }
    let marker = tool.arguments["marker"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let holder = messages
        .iter()
        .find(|m| m.message.to_string().contains(&marker))
        .map(|m| m.mid.clone())
        .ok_or("no stored message holds the tool call")?;
    let Some((start_at, started)) = events.iter().find_map(|&(i, e)| match e {
        LiveEvent::ToolStarted(t) if t.tool_call_id == tool.tool_call_id => Some((i, t)),
        _ => None,
    }) else {
        return Err(format!(
            "no replayed tool_started names tool call {}",
            tool.tool_call_id
        ));
    };
    let arguments: Value = serde_json::from_str(&started.arguments).map_err(|e| {
        format!(
            "tool_started's arguments {:?} are not JSON text: {e}",
            started.arguments
        )
    })?;
    if started.mid != holder
        || started.name != SCRIPTED_TOOL
        || arguments != tool.arguments
        || started.arguments_truncated
    {
        return Err(format!(
            "tool_started is {started:?}: it names the mid of the message holding the call \
             ({holder}), the tool {SCRIPTED_TOOL} and the call's whole arguments"
        ));
    }
    let Some((finish_at, finished)) = events.iter().find_map(|&(i, e)| match e {
        LiveEvent::ToolFinished(t) if t.tool_call_id == tool.tool_call_id => Some((i, t)),
        _ => None,
    }) else {
        return Err(format!(
            "no replayed tool_finished names tool call {}",
            tool.tool_call_id
        ));
    };
    if finish_at < start_at || finished.mid != holder || finished.outcome != "ok" {
        return Err(format!(
            "tool_finished is {finished:?} after tool_started {started:?}: it follows the start, \
             names the same message and the outcome ok"
        ));
    }
    // The runner sent this call to its tool itself, so it recorded when the
    // call started and finished, and reports both.
    match (started.started_at, finished.finished_at) {
        (Some(start), Some(finish)) if start <= finish => {}
        (start, finish) => {
            return Err(format!(
                "tool_started carries started_at {start:?} and tool_finished finished_at \
                 {finish:?} for a call the runner ran itself: it reports both times it \
                 recorded, the finish no earlier than the start"
            ))
        }
    }
    let again = control(&stream(route, SubscribeFrom::Start).await?);
    if again != replay {
        return Err(
            "a second replay from start served other control frames: control events are durable"
                .into(),
        );
    }
    let cursor = replay[start_at]
        .0
        .clone()
        .ok_or("the tool_started control frame carries no cursor")?;
    let after = control(&stream(route, SubscribeFrom::After(cursor)).await?);
    if after[..] != replay[start_at + 1..] {
        return Err(format!(
            "a subscription after the tool_started frame served {} control frames, not the {} \
             that followed it: a reconnect resumes from the durable cursor",
            after.len(),
            replay.len() - start_at - 1
        ));
    }
    Ok(())
}

/// The run has `run_started` and, after it, one `run_finished` with reason
/// `completed`; any `run_paused` for it lies between them.
async fn runs<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, _, run_id) = with_tool_call(case).await?;
    let replay = control(&stream(&session.route, SubscribeFrom::Start).await?);
    let mut started = Vec::new();
    let mut finished = Vec::new();
    let mut paused = Vec::new();
    for (index, (_, event)) in replay.iter().enumerate() {
        match event {
            Some(LiveEvent::RunStarted(run)) if run.run_id == run_id => started.push(index),
            Some(LiveEvent::RunFinished(run)) if run.run_id == run_id => {
                finished.push((index, run.reason.clone()))
            }
            Some(LiveEvent::RunPaused(run)) if run.run_id == run_id => paused.push(index),
            _ => {}
        }
    }
    match (started.as_slice(), finished.as_slice()) {
        ([start], [(finish, reason)])
            if start < finish
                && reason == "completed"
                && paused.iter().all(|p| start < p && p < finish) => {}
        _ => {
            return Err(format!(
                "run {run_id} has run_started at {started:?}, run_finished (with reasons) at \
                 {finished:?} and run_paused at {paused:?}: a completed run has one run_started \
                 and, after it, one run_finished with reason completed"
            ))
        }
    }
    Ok(())
}

/// A session whose next model answer is held after its first `parts`
/// parts streamed, with the send that started it in flight. `None` when
/// the adapter cannot hold an answer mid-stream.
async fn held_stream<S>(
    case: &Case<'_, S>,
    parts: usize,
) -> Result<Option<(Session<S::Route>, Vec<ScriptedPart>, SendReply)>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let turn = vec![
        ScriptedPart::Reasoning(case.mint.next("live-thinking")),
        case.mint.text("live-text").0,
        case.mint.text("live-rest").0,
    ];
    let session = case
        .open(
            "live",
            Script {
                turns: vec![ScriptedTurn::new(turn.clone())],
            },
        )
        .await?;
    if case
        .subject
        .hold_next_model_stream(&session.name, parts)
        .await
        .is_err()
    {
        return Ok(None);
    }
    let send = send_params(
        case.subject,
        &session.name,
        true,
        &case.mint.next("live-prompt"),
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
        .map_err(|e| format!("waiting for the held model stream: {e}"))?;
    Ok(Some((session, turn, reply)))
}

const NO_HELD_STREAM: &str =
    "the adapter cannot hold a model answer mid-stream, so no step is in flight to attach to";

/// The display frames of an attach: `display_reset` first, then one delta
/// per part so far, each with `mid` and `part`, no part twice. Returns the
/// `mid` and each part's `(part, text)`, or `None` before anything shows.
fn replayed(frames: &[Frame]) -> Result<Option<Replay>, String> {
    let display: Vec<&Frame> = frames
        .iter()
        .filter(|f| f.frame.kind == kinds::DISPLAY)
        .collect();
    let Some((first, rest)) = display.split_first() else {
        return Ok(None);
    };
    if first.event != Some(LiveEvent::DisplayReset) {
        return Err(format!(
            "a newly attached stream's first display frame is {:?}: it is display_reset, then \
             one coalesced delta per part of the step so far",
            first.event
        ));
    }
    let mut mid = None;
    let mut parts: Vec<(String, String)> = Vec::new();
    for frame in rest {
        let (delta_mid, part, text) = match &frame.event {
            Some(LiveEvent::TextDelta(d)) | Some(LiveEvent::ReasoningDelta(d)) => {
                (d.mid.clone(), d.part.clone(), d.delta.clone())
            }
            Some(LiveEvent::ToolInputDelta(d)) => (d.mid.clone(), d.part.clone(), d.delta.clone()),
            other => {
                return Err(format!(
                    "after display_reset the attach replay carries {other:?}: only one delta per \
                     part so far"
                ))
            }
        };
        let (Some(delta_mid), Some(part)) = (delta_mid, part) else {
            return Err("an attach-replay delta carries no mid or no part".into());
        };
        if mid.get_or_insert_with(|| delta_mid.clone()) != &delta_mid {
            return Err("the attach replay's deltas name more than one mid".into());
        }
        if parts.iter().any(|(p, _)| *p == part) {
            return Err(format!(
                "the attach replay carries part {part} twice: it is one coalesced delta per part"
            ));
        }
        parts.push((part, text));
    }
    Ok(mid.map(|mid| (mid, parts)))
}

/// Attach while a step is held after two of its parts streamed: the new
/// stream gets `display_reset`, then one coalesced delta per part so far,
/// each the whole part, with the `mid` the stored message then has.
async fn attach<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let Some((session, turn, reply)) = held_stream(case, 2).await? else {
        return Ok(Ending::Inapplicable(NO_HELD_STREAM.into()));
    };
    let texts: Vec<String> = turn[..2]
        .iter()
        .map(|part| match part {
            ScriptedPart::Text(t) | ScriptedPart::Reasoning(t) => t.clone(),
            ScriptedPart::ToolCall(_) => unreachable!("the held turn has no call"),
        })
        .collect();
    let mut found = None;
    let mut last = None;
    for _ in 0..MAX_POLLS {
        let frames = stream(&session.route, SubscribeFrom::Live).await?;
        check_seq("on a stream attached during the step", &frames)?;
        if let Some((mid, parts)) = replayed(&frames)? {
            let shown: Vec<&String> = parts.iter().map(|(_, text)| text).collect();
            if shown == texts.iter().collect::<Vec<_>>() {
                found = Some(mid);
                break;
            }
            last = Some(parts);
        }
        case.subject.pause().await;
    }
    let released = case.subject.release_model_step(&session.name).await;
    let Some(mid) = found else {
        return Err(format!(
            "a stream attached while the step was in flight, after its parts {texts:?} \
             streamed, replayed {last:?}: a newly attached stream first gets display_reset, then \
             one coalesced delta per part so far"
        ));
    };
    released.map_err(|e| format!("releasing the held model stream: {e}"))?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let (_, messages) = read_all(&session.route).await?;
    let stored = messages
        .iter()
        .find(|m| m.message.to_string().contains(&texts[1]))
        .ok_or("no stored message holds the streamed answer")?;
    if stored.mid != mid {
        return Err(format!(
            "the attach replay named mid {mid}, but the answer was stored as {}: a delta's mid \
             is the message's final mid",
            stored.mid
        ));
    }
    Ok(Ending::Passed)
}

/// A stream from start while a step is in flight carries control and
/// display frames, and `seq` rises by one across both.
async fn seq<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let Some((session, _, reply)) = held_stream(case, 1).await? else {
        return Ok(Ending::Inapplicable(NO_HELD_STREAM.into()));
    };
    let mut outcome = Err(
        "a stream from start never carried both control and display frames \
                           while a step was in flight"
            .to_owned(),
    );
    for _ in 0..MAX_POLLS {
        let frames = stream(&session.route, SubscribeFrom::Start).await?;
        let lanes = |kind: &str| frames.iter().any(|f| f.frame.kind == kind);
        if lanes(kinds::CONTROL) && lanes(kinds::DISPLAY) {
            outcome = check_seq("on a stream from start during a step", &frames);
            break;
        }
        case.subject.pause().await;
    }
    case.subject
        .release_model_step(&session.name)
        .await
        .map_err(|e| format!("releasing the held model stream: {e}"))?;
    outcome?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    Ok(Ending::Passed)
}

/// A call replayed from history stored before the runner kept tool-call
/// times. After one run, the adapter seeds a finished call with no stored
/// times; a replay from start carries its `tool_started` and
/// `tool_finished`, both without a time, because the runner recorded none
/// and never invents one (not now, not zero, not the message's time).
async fn untimed<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "live-untimed",
            Script {
                turns: vec![ScriptedTurn::new(vec![
                    case.mint.text("live-untimed-answer").0,
                ])],
            },
        )
        .await?;
    case.run_prompt(&mut session, &case.mint.next("live-untimed-prompt"))
        .await?;
    let call = case.mint.call("live-untimed-call");
    if let Err(e) = case
        .subject
        .seed_untimed_tool_call(case.handle.inner(), &session.name, &call)
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot seed a tool call stored without times: {e}"
        )));
    }
    let replay = control(&stream(&session.route, SubscribeFrom::Start).await?);
    let mut started = None;
    let mut finished = None;
    for (_, event) in replay {
        match event {
            Some(LiveEvent::ToolStarted(t)) if t.tool_call_id == call.tool_call_id => {
                started = Some(t)
            }
            Some(LiveEvent::ToolFinished(t)) if t.tool_call_id == call.tool_call_id => {
                finished = Some(t)
            }
            _ => {}
        }
    }
    let (Some(started), Some(finished)) = (started, finished) else {
        return Err(format!(
            "a replay from start carries no tool_started or no tool_finished for the call {} \
             seeded from history stored without times: every stored call has both",
            call.tool_call_id
        ));
    };
    if started.started_at.is_some() || finished.finished_at.is_some() {
        return Err(format!(
            "the call {} replayed from history stored without times has started_at {:?} and \
             finished_at {:?}: a runner that recorded no time leaves the member out, and never \
             invents one (not now, not zero, not the message's time)",
            call.tool_call_id, started.started_at, finished.finished_at
        ));
    }
    Ok(Ending::Passed)
}

/// Send `prompt` as the owner with a fresh `send_id` and `delivery`, and
/// wait for the run it starts to end. Returns the `send_id` and the run.
async fn send_until_end<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    first: bool,
    prompt: &str,
    delivery: Option<&str>,
) -> Result<(String, String), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let send_id = case.mint.next("send");
    let before = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id);
    let params = send_params(
        case.subject,
        &session.name,
        first,
        prompt,
        &send_id,
        delivery,
    );
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    let end = wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    Ok((send_id, end.run_id))
}

/// The `send_id` the one replayed `run_started` of `run_id` names.
async fn cause_of<R: RunnerRoute>(route: &R, run_id: &str) -> Result<Option<String>, String> {
    let started: Vec<Option<String>> = control(&stream(route, SubscribeFrom::Start).await?)
        .into_iter()
        .filter_map(|(_, event)| match event {
            Some(LiveEvent::RunStarted(run)) if run.run_id == run_id => Some(run.send_id),
            _ => None,
        })
        .collect();
    match started.as_slice() {
        [send_id] => Ok(send_id.clone()),
        other => Err(format!(
            "a replay from start carries {} run_started events for run {run_id}, not one",
            other.len()
        )),
    }
}

/// A session whose model answers each of `turns` runs with one text part.
async fn cause_session<S>(case: &Case<'_, S>, turns: usize) -> Result<Session<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let turns = (0..turns)
        .map(|_| ScriptedTurn::new(vec![case.mint.text("cause-answer").0]))
        .collect();
    case.open("run-cause", Script { turns }).await
}

/// The run a `queue` send started on an idle session names that send's own
/// `send_id` on `run_started`.
async fn queued_cause<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = cause_session(case, 1).await?;
    let prompt = case.mint.next("cause-prompt");
    let (send_id, run_id) = send_until_end(case, &session, true, &prompt, None).await?;
    match cause_of(&session.route, &run_id).await? {
        Some(named) if named == send_id => Ok(()),
        other => Err(format!(
            "run {run_id}, which the queue send {send_id} started, has run_started naming \
             send_id {other:?}: a run a queued send started names that send's own send_id, never \
             an id the runner made for its queue"
        )),
    }
}

/// Every run of a session that only sends started (the admitting send and
/// two later `queue` sends) has a `run_started` carrying a non-empty
/// `send_id`.
async fn every_cause<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = cause_session(case, 3).await?;
    let mut runs = Vec::new();
    for index in 0..3 {
        let prompt = case.mint.next("cause-prompt");
        runs.push(send_until_end(case, &session, index == 0, &prompt, None).await?);
    }
    for (send_id, run_id) in runs {
        match cause_of(&session.route, &run_id).await? {
            Some(named) if !named.is_empty() => {}
            other => {
                return Err(format!(
                    "run {run_id}, which the send {send_id} started, has run_started with send_id \
                     {other:?}: on a run_cause runner a run a send started never omits send_id, \
                     and never sends it empty; only a turn the host started with no send has none"
                ))
            }
        }
    }
    Ok(())
}

/// While a run's model step is held, the owner's `interrupt` starts a run
/// whose `run_started` names the interrupting send, not the send of the run
/// it interrupted.
async fn interrupt_cause<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case.declared.contains(&Capability::Interrupt) {
        return Ok(Ending::Inapplicable(
            "the runner does not declare interrupt".into(),
        ));
    }
    let session = cause_session(case, 3).await?;
    send_until_end(case, &session, true, &case.mint.next("cause-prompt"), None).await?;
    if let Err(e) = case.subject.hold_next_model_step(&session.name).await {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot hold a model step, so there is no running turn to interrupt: {e}"
        )));
    }
    let held_send = case.mint.next("send");
    let params = send_params(
        case.subject,
        &session.name,
        false,
        &case.mint.next("cause-held"),
        &held_send,
        None,
    );
    expect_ok(
        "the send whose model step is held",
        call(&session.route, ops::SESSION_SEND, params).await?,
    )?;
    case.subject
        .await_model_step(&session.name)
        .await
        .map_err(|e| format!("waiting for the held model step: {e}"))?;
    let held_run = head(&session.route)
        .await?
        .last_run_state
        .map(|last| last.run_id)
        .ok_or("the held turn has no run state")?;
    let interrupting = case.mint.next("send");
    let params = send_params(
        case.subject,
        &session.name,
        false,
        &case.mint.next("cause-interrupt"),
        &interrupting,
        Some(groups::INTERRUPT),
    );
    let reply = call(&session.route, ops::SESSION_SEND, params).await;
    // The interrupt aborts the held stream, so releasing it may find
    // nothing left to release; the release only keeps a runner that
    // waited from hanging.
    let _ = case.subject.release_model_step(&session.name).await;
    let reply: SendReply = decode("the interrupt", expect_ok("the interrupt", reply?)?)?;
    let end = wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        Some(&held_run),
        &reply,
    )
    .await?;
    match cause_of(&session.route, &end.run_id).await? {
        Some(named) if named == interrupting => Ok(Ending::Passed),
        other => Err(format!(
            "run {}, which the interrupt {interrupting} started, has run_started naming send_id \
             {other:?} (the interrupted run {held_run} was started by {held_send}): it names the \
             interrupting send, never the send of the run it interrupted",
            end.run_id
        )),
    }
}
