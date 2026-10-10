//! Compaction cases observe requests received by the scripted provider and
//! model, and read the runner's real session routes. The runner adapter must
//! inject the clock into runner timers, not infer elapsed time from logs.
//! Compaction terms are defined in the crate's Contract vocabulary section.

use std::{collections::BTreeSet, path::Path};

use cortexkit_role_compaction_provider::errors::RefuseCode;
use cortexkit_role_harness::{CrashDriver, KillPoint, Trigger};
use cortexkit_role_llm_runner::{
    ops, points,
    read::{EntrySource, ModelEntry, ModelPage, ReadRequest, ReadView},
    run::RunState,
    send::SendReply,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Session},
    drive::{
        call, decode, expect_ok, head as session_head, read_all, run_result, send_params, Mint,
    },
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};

/// A provider replacement in schema-neutral form. The runner adapter must
/// translate only replacement text to its runner's message schema, preserving
/// the range, ids and version verbatim.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionMessage {
    pub compaction_id: String,
    pub version: u64,
    pub from: u64,
    pub to: u64,
    pub replacement: Vec<String>,
}

/// A step answer the runner adapter must translate to compaction-provider/v1
/// without changing the answer kind or fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompactionAnswer {
    Noop,
    Message(CompactionMessage),
    Wait {
        reason: String,
        bound_ms: u64,
    },
    Refuse {
        code: RefuseCode,
        reason: String,
        /// The finer code carried in the provider's wire answer.
        provider_code: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompactionSetup {
    Ready(CompactionMessage),
    /// Fail the actual provider call without an answer, not a REFUSE answer.
    Unavailable,
    Refuse {
        code: RefuseCode,
        reason: String,
        provider_code: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompactionStep {
    Answer(CompactionAnswer),
    /// Prepare this answer in the scripted provider, but deliver nothing
    /// until the case calls [`LlmRunnerSubject::answer_compaction`]. Preparing
    /// content does not authorize the runner to apply it before delivery.
    Hold(CompactionAnswer),
}

/// Configuration for the scripted provider, held model calls and runner clock.
/// The runner adapter must advance time only through
/// [`LlmRunnerSubject::advance_compaction_clock`]. Setup must omit `call_when`
/// so the runner calls the provider on every step. A step without a configured
/// answer must receive NOOP. The adapter must deliver timed-out and duplicate
/// answers to the runner; only the runner may enforce its answer fence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionScript {
    pub setup: CompactionSetup,
    pub steps: Vec<CompactionStep>,
    pub call_timeout_ms: u64,
    pub wait_cap_ms: u64,
    pub prompt_cache_lifetime_ms: u64,
    pub context_window: u64,
    pub output_limit: u64,
    pub safety_margin: u64,
    /// Configure the runner's estimator to report this input token count.
    pub request_tokens: u64,
}

impl CompactionScript {
    pub fn new(initial: CompactionMessage) -> Self {
        Self {
            setup: CompactionSetup::Ready(initial),
            steps: Vec::new(),
            call_timeout_ms: 25,
            wait_cap_ms: 100,
            prompt_cache_lifetime_ms: 100,
            context_window: 1000,
            output_limit: 100,
            safety_margin: 50,
            request_tokens: 100,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactionCallKind {
    Setup,
    Step,
}

/// A call captured when the scripted provider receives a runner request.
/// The runner adapter must not fabricate calls from the configured script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionCall {
    pub kind: CompactionCallKind,
    pub request_id: String,
    /// Times on the injected clock, measured from initial installation at zero.
    pub issued_at_ms: u64,
    pub deadline_ms: u64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactionObservation {
    pub calls: Vec<CompactionCall>,
    /// Canonical messages captured from requests received by the scripted
    /// model while its answers are held. The runner adapter must translate
    /// each request to ModelEntry, preserving its content and sources; it must
    /// not substitute `session.read` responses or configured replacements.
    pub model_inputs: Vec<Vec<crate::wire::read::ModelEntry>>,
}

fn initial(marker: &str) -> CompactionMessage {
    CompactionMessage {
        compaction_id: marker.into(),
        version: 1,
        from: 0,
        to: 0,
        replacement: vec![format!("{marker}-head"), format!("{marker}-note")],
    }
}

fn summary(marker: &str, version: u64, to: u64) -> CompactionMessage {
    CompactionMessage {
        compaction_id: marker.into(),
        version,
        from: 0,
        to,
        replacement: vec![format!("{marker}-summary"), format!("{marker}-detail")],
    }
}

fn text(value: &str) -> ScriptedTurn {
    ScriptedTurn {
        parts: vec![ScriptedPart::Text(value.into())],
    }
}

async fn observation<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
) -> Result<CompactionObservation, String> {
    subject
        .compaction_observation(session)
        .await
        .map_err(|e| e.to_string())
}

async fn model_page<R: RunnerRoute>(route: &R, from: u64, caps: bool) -> Result<ModelPage, String> {
    let mut request = ReadRequest::range(from);
    request.view = Some(ReadView::Model);
    let mut params = serde_json::to_value(request).expect("serializes");
    if caps {
        params["limit"] = json!(1);
        params["max_bytes"] = json!(1);
    }
    let page: ModelPage = decode(
        "model session.read",
        expect_ok(
            "model session.read",
            call(route, ops::SESSION_READ, params).await?,
        )?,
    )?;
    page.check(Some(from))
        .map_err(|e| format!("malformed model page: {e:?}"))?;
    Ok(page)
}

fn expect_message(page: &ModelPage, expected: &CompactionMessage) -> Result<(), String> {
    if page.compaction_id.as_deref() != Some(&expected.compaction_id)
        || page.version != Some(expected.version)
    {
        return Err(format!(
            "view names {:?}/{:?}, expected {}/{}",
            page.compaction_id, page.version, expected.compaction_id, expected.version
        ));
    }
    let replacements: Vec<_> = page
        .messages
        .iter()
        .filter(|e| matches!(e.source, EntrySource::Replacement { .. }))
        .collect();
    if replacements.len() != expected.replacement.len() {
        return Err(format!(
            "{} replacement entries, expected {}",
            replacements.len(),
            expected.replacement.len()
        ));
    }
    for (entry, marker) in replacements.iter().zip(&expected.replacement) {
        let expected_source = EntrySource::Replacement {
            compaction_id: expected.compaction_id.clone(),
            version: expected.version,
            from_ordinal: expected.from,
            to_ordinal: expected.to,
        };
        if entry.source != expected_source || !entry.message.to_string().contains(marker) {
            return Err(format!(
                "replacement {entry:?} does not carry {expected_source:?} and {marker}"
            ));
        }
    }
    Ok(())
}

async fn install<S>(
    case: &Case<'_, S>,
    label: &str,
    script: CompactionScript,
    model: Script,
) -> Result<Session<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open(label, model).await?;
    case.subject
        .install_compaction_script(&session.name, script)
        .await
        .map_err(|e| e.to_string())?;
    Ok(session)
}

async fn send<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    prompt: &str,
) -> Result<SendReply, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let id = case.mint.next("send");
    let reply = decode(
        "compaction send",
        expect_ok(
            "compaction send",
            call(
                &session.route,
                ops::SESSION_SEND,
                case.params(session, prompt, &id, None),
            )
            .await?,
        )?,
    )?;
    if session.first.is_none() {
        session.first = Some((id, prompt.into()));
    }
    Ok(reply)
}

async fn release_model<S: LlmRunnerSubject>(subject: &S, session: &str) -> Result<(), String> {
    subject
        .release_compaction_model(session)
        .await
        .map_err(|e| e.to_string())
}

async fn answer<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
    index: usize,
    request: Option<String>,
    value: CompactionAnswer,
) -> Result<(), String> {
    subject
        .answer_compaction(session, index, request, value)
        .await
        .map_err(|e| e.to_string())
}

async fn advance<S: LlmRunnerSubject>(subject: &S, session: &str, ms: u64) -> Result<(), String> {
    subject
        .advance_compaction_clock(session, ms)
        .await
        .map_err(|e| e.to_string())
}

/// The run id for a send: the id in the send's reply, or, when the reply names
/// none, the session's newest run, which is the one this send started.
async fn send_run_id<R: RunnerRoute>(
    session: &Session<R>,
    reply: &SendReply,
) -> Result<String, String> {
    Ok(match &reply.run_id {
        Some(id) => id.clone(),
        None => {
            crate::drive::head(&session.route)
                .await?
                .last_run_state
                .ok_or("send has no observable run")?
                .run_id
        }
    })
}

/// One read of the run's current state. Only for a case that checks the run has
/// NOT ended yet; to assert how a run ended, use [`result`].
async fn current_result<R: RunnerRoute>(
    session: &Session<R>,
    reply: &SendReply,
) -> Result<crate::wire::run::RunResult, String> {
    run_result(&session.route, &send_run_id(session, reply).await?).await
}

/// The result of the run a send started, once that run has ended. A run that
/// has not ended yet answers its current state (`active`), so a single read
/// can see `active` for a run that is about to end in `error`.
async fn result<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    reply: &SendReply,
) -> Result<crate::wire::run::RunResult, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let id = send_run_id(session, reply).await?;
    for _ in 0..crate::drive::MAX_POLLS {
        let result = run_result(&session.route, &id).await?;
        if result.run_state().is_terminal() {
            return Ok(result);
        }
        case.subject.pause().await;
    }
    Err(format!(
        "run {id} did not end within {} polls",
        crate::drive::MAX_POLLS
    ))
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "model_view_half_open_ranges_and_travel" => ranges(case).await,
        "compaction_unavailable_distinct_from_refuse" => refusals(case).await,
        "compaction_wait_cap" => wait_cap(case).await,
        "compaction_late_or_stale_answers_discarded" => late(case).await,
        "compaction_step_timeout_uses_last_view" => step_timeout(case).await,
        "compaction_one_view_per_step" => one_view(case).await,
        _ => Err(format!("unknown compaction case {name}")),
    }
}

async fn ranges<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let head = initial(&case.mint.next("ranges"));
    let compact = summary(&head.compaction_id, 2, 2);
    let mut script = CompactionScript::new(head.clone());
    script.steps = vec![
        CompactionStep::Answer(CompactionAnswer::Noop),
        CompactionStep::Hold(CompactionAnswer::Message(compact.clone())),
    ];
    let mut session = install(
        case,
        "ranges",
        script,
        Script {
            turns: vec![text("first-answer"), text("second-answer")],
        },
    )
    .await?;
    send(case, &mut session, "first-prompt").await?;
    release_model(case.subject, &session.name).await?;
    let (_, raw) = read_all(&session.route).await?;
    if raw.len() != 2 {
        return Err(format!(
            "range fixture needs two raw messages, got {}",
            raw.len()
        ));
    }
    let first = model_page(&session.route, 0, true).await?;
    expect_message(&first, &head)?;
    if first.messages.len() != 3
        || first.messages[2].source
            != (EntrySource::Message {
                ordinal: 0,
                mid: raw[0].mid.clone(),
            })
        || first.next_from_ordinal != Some(1)
    {
        return Err(format!(
            "head insertions did not travel before message zero over both caps: {first:?}"
        ));
    }
    let next = model_page(&session.route, 1, true).await?;
    if next.messages.len() != 1
        || next.messages[0].source.from_ordinal() != 1
        || next.messages[0].source.is_insertion()
    {
        return Err(format!(
            "head insertion repeated on the next page: {next:?}"
        ));
    }
    send(case, &mut session, "exclusive-end-prompt").await?;
    let obs = observation(case.subject, &session.name).await?;
    let index = obs.calls.len().checked_sub(1).ok_or("no summary request")?;
    answer(
        case.subject,
        &session.name,
        index,
        None,
        CompactionAnswer::Message(compact.clone()),
    )
    .await?;
    let page = model_page(&session.route, 0, true).await?;
    expect_message(&page, &compact)?;
    if page.messages.len() != 2 || page.next_from_ordinal != Some(2) {
        return Err(format!(
            "summary split over caps or cursor not at exclusive end: {page:?}"
        ));
    }
    for from in [1, 2] {
        let tail = model_page(&session.route, from, true).await?;
        if tail.messages.len() != 1
            || !matches!(
                tail.messages[0].source,
                EntrySource::Message { ordinal: 2, .. }
            )
            || !tail.messages[0]
                .message
                .to_string()
                .contains("exclusive-end-prompt")
            || tail.next_from_ordinal.is_some()
        {
            return Err(format!(
                "half-open end lost or summary repeated from {from}: {tail:?}"
            ));
        }
    }
    let obs = observation(case.subject, &session.name).await?;
    if obs.model_inputs.len() != 2 {
        return Err(format!(
            "expected two held model inputs, got {}",
            obs.model_inputs.len()
        ));
    }
    if obs.model_inputs[1] != model_page(&session.route, 0, false).await?.messages {
        return Err("model request differs from canonical summarized view".into());
    }
    Ok(())
}

/// Check `RunResult.error`'s refusal fields against the configured role code,
/// reason and finer code. Other error fields use the runner's own schema.
fn refusal_error(
    error: &Value,
    code: &str,
    reason: &str,
    finer: Option<&str>,
) -> Result<(), String> {
    if error["provider_code"] != code
        || error["reason"] != reason
        || error["provider"].as_str().is_none_or(str::is_empty)
        || error.get("provider_detail_code").and_then(Value::as_str) != finer
    {
        return Err(format!(
            "REFUSE mapping lost role code/provider/reason or finer diagnostic: {error}"
        ));
    }
    Ok(())
}

async fn refusals<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for setup in [
        CompactionSetup::Unavailable,
        CompactionSetup::Refuse {
            code: RefuseCode::Misconfigured,
            reason: "fix compaction configuration".into(),
            provider_code: Some("diagnostic-only".into()),
        },
        CompactionSetup::Refuse {
            code: RefuseCode::HistoryUnreadable,
            reason: "history unavailable".into(),
            provider_code: Some("history-diagnostic".into()),
        },
        CompactionSetup::Refuse {
            code: RefuseCode::Unknown("future-role-code".into()),
            reason: "unknown preserved".into(),
            provider_code: None,
        },
    ] {
        let mut script = CompactionScript::new(initial(&case.mint.next("refusal")));
        script.setup = setup.clone();
        let mut session = install(
            case,
            "refusals",
            script.clone(),
            Script {
                turns: vec![text("usable-again")],
            },
        )
        .await?;
        let reply = send(case, &mut session, "refused-prompt").await?;
        let ended = result(case, &session, &reply).await?;
        if ended.run_state() != RunState::Error {
            return Err(format!("Setup failure ended {:?}, not error", ended.state));
        }
        let error = ended.error.ok_or("Setup failure has no run error")?;
        match &setup {
            CompactionSetup::Unavailable => {
                if error["provider_code"] != "compaction_unavailable"
                    || error.get("provider_detail_code").is_some()
                {
                    return Err(format!(
                        "unavailable was mapped to a provider refusal: {error}"
                    ));
                }
            }
            CompactionSetup::Refuse {
                code,
                reason,
                provider_code,
            } => refusal_error(&error, code.as_str(), reason, provider_code.as_deref())?,
            CompactionSetup::Ready(_) => unreachable!(),
        }
        let obs = observation(case.subject, &session.name).await?;
        if !obs.model_inputs.is_empty() {
            return Err("Setup failure sent a model request".into());
        }
        let (_, raw) = read_all(&session.route).await?;
        if raw.iter().any(|m| {
            m.message.to_string().contains("diagnostic-only")
                || m.message
                    .to_string()
                    .contains("fix compaction configuration")
        }) {
            return Err("provider refusal added content to history".into());
        }
        // Configure a successful Setup answer in the external provider without
        // changing the runner's stored session records or in-memory session.
        script.setup = CompactionSetup::Ready(initial(&case.mint.next("retry-head")));
        case.subject
            .install_compaction_script(&session.name, script)
            .await
            .map_err(|e| e.to_string())?;
        send(case, &mut session, "retry-after-refusal").await?;
        release_model(case.subject, &session.name).await?;
        let obs = observation(case.subject, &session.name).await?;
        if obs
            .calls
            .iter()
            .filter(|c| c.kind == CompactionCallKind::Setup)
            .count()
            != 2
            || obs.model_inputs.len() != 1
        {
            return Err(
                "session unusable or Setup not retried after failed/refused initial answer".into(),
            );
        }
    }
    // Check role-code and finer-code run-error fields for a step REFUSE as
    // well as for a Setup REFUSE.
    let code = RefuseCode::WindowTooSmall;
    let reason = "step cannot reduce enough";
    let mut script = CompactionScript::new(initial(&case.mint.next("step-refusal")));
    script.steps = vec![CompactionStep::Answer(CompactionAnswer::Refuse {
        code: code.clone(),
        reason: reason.into(),
        provider_code: Some("step-diagnostic".into()),
    })];
    let mut session = install(
        case,
        "step-refusal",
        script,
        Script {
            turns: vec![text("not-sent")],
        },
    )
    .await?;
    let reply = send(case, &mut session, "step-refused-prompt").await?;
    let ended = result(case, &session, &reply).await?;
    if ended.run_state() != RunState::Error {
        return Err("step REFUSE did not end error".into());
    }
    refusal_error(
        &ended.error.ok_or("step REFUSE has no error")?,
        code.as_str(),
        reason,
        Some("step-diagnostic"),
    )?;
    if !observation(case.subject, &session.name)
        .await?
        .model_inputs
        .is_empty()
    {
        return Err("step REFUSE sent a model request".into());
    }
    Ok(())
}

async fn wait_cap<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut script = CompactionScript::new(initial(&case.mint.next("wait")));
    // The 875-token input estimate plus the 100-token output allowance fits
    // the 1000-token window, but exceeds the safe limit after reserving the
    // 50-token safety margin: 875 + 100 > 1000 - 50.
    script.request_tokens = 875;
    script.steps = vec![CompactionStep::Answer(CompactionAnswer::Wait {
        reason: "building summary".into(),
        bound_ms: 200,
    })];
    let cap = script.wait_cap_ms.min(script.prompt_cache_lifetime_ms);
    let mut session = install(
        case,
        "wait",
        script,
        Script {
            turns: vec![text("must-not-be-sent")],
        },
    )
    .await?;
    let reply = send(case, &mut session, "near-window").await?;
    advance(case.subject, &session.name, cap - 1).await?;
    if current_result(&session, &reply)
        .await?
        .run_state()
        .is_terminal()
    {
        return Err("WAIT ended before its cap".into());
    }
    if !observation(case.subject, &session.name)
        .await?
        .model_inputs
        .is_empty()
    {
        return Err("WAIT sent a model request before its cap".into());
    }
    advance(case.subject, &session.name, 1).await?;
    let ended = result(case, &session, &reply).await?;
    if ended.run_state() != RunState::Error
        || ended
            .error
            .as_ref()
            .is_none_or(|e| e["provider_code"] != "compaction_wait_exceeded")
    {
        return Err(format!(
            "WAIT did not end compaction_wait_exceeded at {cap} ms: {ended:?}"
        ));
    }
    if !observation(case.subject, &session.name)
        .await?
        .model_inputs
        .is_empty()
    {
        return Err("over-margin request sent at WAIT cap".into());
    }
    Ok(())
}

async fn step_timeout<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let head = initial(&case.mint.next("timeout"));
    let mut script = CompactionScript::new(head.clone());
    script.steps = vec![CompactionStep::Hold(CompactionAnswer::Noop)];
    let timeout = script.call_timeout_ms;
    let mut session = install(
        case,
        "timeout",
        script,
        Script {
            turns: vec![text("after-timeout")],
        },
    )
    .await?;
    let reply = send(case, &mut session, "timeout-prompt").await?;
    advance(case.subject, &session.name, timeout - 1).await?;
    if !observation(case.subject, &session.name)
        .await?
        .model_inputs
        .is_empty()
    {
        return Err("step sent before call timeout".into());
    }
    advance(case.subject, &session.name, 1).await?;
    let obs = observation(case.subject, &session.name).await?;
    if obs.model_inputs.len() != 1 {
        return Err("step timeout did not proceed with last view".into());
    }
    let page = model_page(&session.route, 0, false).await?;
    expect_message(&page, &head)?;
    if obs.model_inputs[0] != page.messages {
        return Err("timeout model input differs from last applied view".into());
    }
    release_model(case.subject, &session.name).await?;
    if result(case, &session, &reply).await?.run_state() != RunState::Completed {
        return Err("step timeout ended a run error instead of continuing".into());
    }
    Ok(())
}

async fn late<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for extra in [0, 1] {
        let head = initial(&case.mint.next("late"));
        let mut script = CompactionScript::new(head.clone());
        script.steps = vec![
            CompactionStep::Hold(CompactionAnswer::Noop),
            CompactionStep::Hold(CompactionAnswer::Noop),
        ];
        let timeout = script.call_timeout_ms;
        let tool = case.mint.call("late-tool");
        let mut session = install(
            case,
            "late",
            script,
            Script {
                turns: vec![
                    ScriptedTurn {
                        parts: vec![ScriptedPart::ToolCall(tool)],
                    },
                    text("late-final"),
                ],
            },
        )
        .await?;
        send(case, &mut session, "late-prompt").await?;
        let obs = observation(case.subject, &session.name).await?;
        if obs.calls.len() != 2 {
            return Err("expected Setup and held first step".into());
        }
        advance(case.subject, &session.name, timeout + extra).await?;
        answer(
            case.subject,
            &session.name,
            1,
            None,
            CompactionAnswer::Message(summary(&head.compaction_id, 99, 1)),
        )
        .await?;
        expect_message(&model_page(&session.route, 0, false).await?, &head)?;
        if observation(case.subject, &session.name)
            .await?
            .model_inputs
            .is_empty()
        {
            // Check rejection of late answers even if the timed-out run has
            // ended. When no held model call is left to release, the case sends
            // another message into the session, which makes the runner issue a
            // fresh request. compaction_step_timeout_uses_last_view
            // checks continuation after timeout separately.
            send(case, &mut session, "late-followup").await?;
        } else {
            release_model(case.subject, &session.name).await?;
        }
        let obs = observation(case.subject, &session.name).await?;
        if obs.calls.len() != 3 || obs.calls[2].request_id == obs.calls[1].request_id {
            return Err("fresh step request not observed".into());
        }
        // Deliver before the fresh request's deadline, but name the previous
        // request's id, so only the request fence can reject this answer.
        answer(
            case.subject,
            &session.name,
            2,
            Some(obs.calls[1].request_id.clone()),
            CompactionAnswer::Message(summary(&head.compaction_id, 100, 1)),
        )
        .await?;
        expect_message(&model_page(&session.route, 0, false).await?, &head)?;
        answer(case.subject, &session.name, 2, None, CompactionAnswer::Noop).await?;
        let obs = observation(case.subject, &session.name).await?;
        if obs.model_inputs.is_empty()
            || obs.model_inputs.iter().flatten().any(|e| {
                matches!(
                    e.source,
                    EntrySource::Replacement {
                        version: 99 | 100,
                        ..
                    }
                )
            })
        {
            return Err("late/stale answer reached a model request".into());
        }
    }
    Ok(())
}

async fn one_view<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let head = initial(&case.mint.next("single-view"));
    let first = summary(&head.compaction_id, 2, 1);
    let mut script = CompactionScript::new(head.clone());
    script.steps = vec![CompactionStep::Hold(CompactionAnswer::Message(
        first.clone(),
    ))];
    let mut session = install(
        case,
        "single-view",
        script,
        Script {
            turns: vec![text("single-final")],
        },
    )
    .await?;
    send(case, &mut session, "single-prompt").await?;
    answer(
        case.subject,
        &session.name,
        1,
        None,
        CompactionAnswer::Message(first.clone()),
    )
    .await?;
    // Deliver a second view for the current request before its deadline,
    // with a higher version. The consumed answer fence must reject it.
    answer(
        case.subject,
        &session.name,
        1,
        None,
        CompactionAnswer::Message(summary(&head.compaction_id, 3, 1)),
    )
    .await?;
    expect_message(&model_page(&session.route, 0, false).await?, &first)?;
    let obs = observation(case.subject, &session.name).await?;
    if obs.model_inputs.len() != 1
        || obs.model_inputs[0] != model_page(&session.route, 0, false).await?.messages
    {
        return Err("second view applied within one step".into());
    }
    Ok(())
}

/// Run durability checks in separate persistent state directories and record
/// kills in the conformance driver's ledger. Kill at FoldRecorded for Setup
/// and at CompactionApplied for the step answer. Each point is killed once
/// per conformance run.
pub async fn crash_case<S>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
    name: &str,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let root = work_dir.join(name);
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let head = initial(&mint.next("crash-head"));
    let compact = summary(&head.compaction_id, 2, 3);
    let fence = name == "compaction_fence_crash_replays_model_view";
    let mut script = CompactionScript::new(head.clone());
    let model = if fence {
        script.steps = vec![
            CompactionStep::Answer(CompactionAnswer::Noop),
            CompactionStep::Hold(CompactionAnswer::Message(compact.clone())),
        ];
        Script {
            turns: vec![
                ScriptedTurn {
                    parts: vec![ScriptedPart::ToolCall(mint.call("fence-tool"))],
                },
                text("fence-final"),
            ],
        }
    } else {
        Script {
            turns: vec![text("setup-final"), text("followup-final")],
        }
    };
    let handle = driver.spawn(&root).await.map_err(|e| e.to_string())?;
    let case = Case {
        subject,
        handle: &handle,
        declared,
        mint,
    };
    let mut live = install(&case, "compaction-control", script.clone(), model.clone()).await?;
    send(&case, &mut live, "same-prompt").await?;
    if fence {
        release_model(subject, &live.name).await?;
        expect_message(&model_page(&live.route, 0, false).await?, &head)?;
        answer(
            subject,
            &live.name,
            2,
            None,
            CompactionAnswer::Message(compact.clone()),
        )
        .await?;
    }
    let expected = model_page(&live.route, 0, false).await?;
    let obs = observation(subject, &live.name).await?;
    if obs.model_inputs.is_empty() {
        return Err("control never called the model".into());
    }
    let durable = subject
        .durable_compaction_setup(handle.inner(), &live.name)
        .await
        .map_err(|e| e.to_string())?;
    if durable.as_ref() != Some(&head) {
        return Err("Setup not durable when the first model call was observed".into());
    }
    // Keep the model held; no assistant answer can race the crash comparison.
    let mut cut = install(&case, "compaction-cut", script, model).await?;
    let params = send_params(
        subject,
        &cut.name,
        true,
        "same-prompt",
        &mint.next("send"),
        None,
    );
    if fence {
        send(&case, &mut cut, "same-prompt").await?;
        release_model(subject, &cut.name).await?;
        expect_message(&model_page(&cut.route, 0, false).await?, &head)?;
    }
    let cut_name = cut.name.clone();
    let route = &cut.route;
    let trigger: Trigger<'_> = if fence {
        let compact = compact.clone();
        let cut_name = cut_name.clone();
        Box::pin(async move {
            let _ = subject
                .answer_compaction(&cut_name, 2, None, CompactionAnswer::Message(compact))
                .await;
        })
    } else {
        Box::pin(async move {
            let _ = route.request(ops::SESSION_SEND, params).await;
        })
    };
    drop(live);
    let point = if fence {
        points::COMPACTION_APPLIED
    } else {
        points::FOLD_RECORDED
    };
    driver
        .kill_at(handle, &KillPoint::new(point), trigger)
        .await
        .map_err(|e| e.to_string())?;
    drop(cut);
    let handle = driver.restart(&root).await.map_err(|e| e.to_string())?;
    // Reopen cut_name after restart so the provider retains the session's
    // captured calls and configured answers under the original identity.
    if fence {
        finish_crash(subject, &handle, &cut_name, &head, &expected, true).await?;
    } else {
        finish_setup_crash(subject, &handle, &cut_name, &head, &expected, mint).await?;
        let obs = observation(subject, &cut_name).await?;
        if obs.model_inputs.len() != 1 {
            return Err("restarted Setup did not reach its first held model request".into());
        }
        release_model(subject, &cut_name).await?;
        let route = subject
            .session_route(handle.inner(), &cut_name, &subject.owner_stamp())
            .await
            .map_err(|e| e.to_string())?;
        expect_ok(
            "post-Setup follow-up",
            call(
                &route,
                ops::SESSION_SEND,
                send_params(
                    subject,
                    &cut_name,
                    false,
                    "follow-up-without-setup",
                    &mint.next("send"),
                    None,
                ),
            )
            .await?,
        )?;
        let obs = observation(subject, &cut_name).await?;
        if obs
            .calls
            .iter()
            .filter(|c| c.kind == CompactionCallKind::Setup)
            .count()
            != 1
            || obs.model_inputs.len() != 2
        {
            return Err("Setup re-ran or stopped being usable on a later send".into());
        }
    }
    Ok(())
}

const SETUP_RESUME_PROMPT: &str = "resume-after-recorded-compaction-setup";

fn expect_crash_entries(
    what: &str,
    actual: &[ModelEntry],
    expected: &[ModelEntry],
) -> Result<(), String> {
    if actual.len() != expected.len() {
        return Err(format!(
            "{what}: message count differs: {} versus {} (only the resume user message may be appended)",
            actual.len(), expected.len()
        ));
    }
    for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
        if actual.source != expected.source {
            return Err(format!(
                "{what}: source at message {index} differs (ids, range or ordering): {:?} versus {:?}",
                actual.source, expected.source
            ));
        }
        if actual.message != expected.message {
            return Err(format!(
                "{what}: content at message {index} differs: {} versus {}",
                actual.message, expected.message
            ));
        }
    }
    Ok(())
}

async fn finish_setup_crash<S>(
    subject: &S,
    handle: &crate::harness::DriverHandle<S>,
    session: &str,
    head: &CompactionMessage,
    expected: &ModelPage,
    mint: &Mint,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let route = subject
        .session_route(handle.inner(), session, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    let last = session_head(&route)
        .await?
        .last_run_state
        .ok_or("Setup crash recovery has no run state")?;
    let interrupted = match RunState::parse(&last.state) {
        RunState::Active => false,
        RunState::Interrupted => true,
        _ => {
            return Err(format!(
                "Setup crash recovery has unexpected run state {} (expected active or interrupted)",
                last.state
            ));
        }
    };
    finish_crash(subject, handle, session, head, expected, false).await?;
    let mut expected = model_page(&route, 0, false).await?;
    if interrupted {
        let obs = observation(subject, session).await?;
        if !obs.model_inputs.is_empty() {
            return Err("sealed Setup crash run called the model before a resume send".into());
        }
        // The uncrashed comparison run used its own session, so its message ids
        // can't be compared with this one. Instead, compare this session's view
        // with itself: what the model sees after the resume send must be the
        // recovered view plus the one new user message, and nothing else.
        let (_, before) = read_all(&route).await?;
        let reply: SendReply = decode(
            "Setup crash resume send",
            expect_ok(
                "Setup crash resume send",
                call(
                    &route,
                    ops::SESSION_SEND,
                    send_params(
                        subject,
                        session,
                        false,
                        SETUP_RESUME_PROMPT,
                        &mint.next("send"),
                        None,
                    ),
                )
                .await?,
            )?,
        )?;
        let (_, after) = read_all(&route).await?;
        if after.len() != before.len() + 1 || after[..before.len()] != before {
            return Err(
                "Setup resume changed the kept transcript outside the appended user message".into(),
            );
        }
        let user = after.last().expect("one message appended");
        let run_id = reply
            .run_id
            .as_ref()
            .ok_or("Setup resume send has no run_id")?;
        // Recognise the appended message by its prompt text and by the send
        // having started a new run. Don't also require the message to name its
        // run: that per-message attribution is a separate rule of the runner
        // contract (section 6, run attribution) with its own case, and this
        // case only checks that Setup survives a crash without re-running.
        if !user.message.to_string().contains(SETUP_RESUME_PROMPT) || run_id == &last.run_id {
            return Err("Setup resume did not append the new send's user message".into());
        }
        expected.messages.push(ModelEntry::new(
            EntrySource::Message {
                ordinal: user.ordinal,
                mid: user.mid.clone(),
            },
            user.message.clone(),
        ));
    }
    let obs = observation(subject, session).await?;
    if obs
        .calls
        .iter()
        .filter(|c| c.kind == CompactionCallKind::Setup)
        .count()
        != 1
    {
        return Err("Setup was re-run after its durable answer on crash recovery".into());
    }
    let page = model_page(&route, 0, false).await?;
    if page.compaction_id != expected.compaction_id || page.version != expected.version {
        return Err("Setup crash recovery changed the compaction id or version outside the appended user message".into());
    }
    if page.lineage_id != expected.lineage_id
        || page.next_from_ordinal != expected.next_from_ordinal
    {
        return Err("Setup crash recovery changed the lineage or page cursor".into());
    }
    expect_crash_entries(
        "Setup recovered model page",
        &page.messages,
        &expected.messages,
    )?;
    if obs.model_inputs.len() != 1 {
        return Err("Setup crash recovery did not reach exactly one held model request".into());
    }
    expect_crash_entries(
        "Setup first held model request",
        &obs.model_inputs[0],
        &expected.messages,
    )
}

async fn finish_crash<S>(
    subject: &S,
    handle: &crate::harness::DriverHandle<S>,
    session: &str,
    head: &CompactionMessage,
    expected: &ModelPage,
    fence: bool,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let route = subject
        .session_route(handle.inner(), session, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    let page = model_page(&route, 0, false).await?;
    let normalize = |page: &ModelPage| -> Vec<Value> {
        page.messages
            .iter()
            .map(|e| {
                let mut source = serde_json::to_value(&e.source).expect("serializes");
                if let Some(obj) = source.as_object_mut() {
                    obj.remove("mid");
                }
                // Message schemas may contain session-specific metadata. Compare
                // the known scripted content, range, order and entry count rather
                // than demanding identical timestamps/ids on independent roots.
                let markers = [
                    "same-prompt".to_owned(),
                    format!("{}-head", head.compaction_id),
                    format!("{}-note", head.compaction_id),
                    format!("{}-summary", head.compaction_id),
                    format!("{}-detail", head.compaction_id),
                ];
                let content: Vec<_> = markers
                    .iter()
                    .filter(|m| e.message.to_string().contains(m.as_str()))
                    .collect();
                json!({"source":source,"content":content})
            })
            .collect()
    };
    if normalize(&page) != normalize(expected)
        || page.compaction_id != expected.compaction_id
        || page.version != expected.version
    {
        return Err(format!(
            "crash view differs from uncrashed view: {page:?} versus {expected:?}"
        ));
    }
    let durable = subject
        .durable_compaction_setup(handle.inner(), session)
        .await
        .map_err(|e| e.to_string())?;
    if durable.as_ref() != Some(head) {
        return Err("initial answer lost across crash".into());
    }
    let obs = observation(subject, session).await?;
    if obs
        .calls
        .iter()
        .filter(|c| c.kind == CompactionCallKind::Setup)
        .count()
        != 1
    {
        return Err("Setup was re-run after its durable answer".into());
    }
    if fence && obs.calls.len() != 3 {
        return Err("recorded compaction step was re-invoked on replay".into());
    }
    Ok(())
}
