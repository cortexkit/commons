//! Compaction cases observe the Setup calls received by the scripted
//! provider and the requests received by the scripted model, and read the
//! runner's real session routes. They check Setup only: the runner half of
//! compaction-provider 0.3.0 §3 (notices, views, rides, the ceiling, native
//! markers) has no live case yet. Compaction terms are defined in the
//! crate's Contract vocabulary section.

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

/// What the scripted provider answers the runner's Setup call with. The
/// runner adapter translates it to compaction-provider/v1 without changing
/// its kind or fields.
///
/// Non-exhaustive: a later answer kind is additive, so an adapter's `match`
/// keeps a `_` arm (answering an unknown kind as a failed Setup, say).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CompactionSetup {
    Ready(CompactionMessage),
    /// Answer `ready` with `initial` and a Setup checkpoint
    /// `{tag_checkpoint, cut}` whose `cut` is the message the Setup request
    /// names as `newest` (the session's first message, its prompt), as a
    /// provider does for a session it moved from standalone Magic Context.
    ReadyWithCheckpoint {
        initial: CompactionMessage,
        tag_checkpoint: u64,
    },
    /// Answer `ready` with `initial` and the `cache_ttl` entry
    /// (compaction-provider §8.1), whatever model it names.
    ReadyWithCacheTtl {
        initial: CompactionMessage,
        cache_ttl: cortexkit_role_compaction_provider::cache_ttl::CacheTtl,
    },
    /// Answer `ready` with `initial`, the `cache_ttl` entry when given, and
    /// `ride_wait_ms` (compaction-provider §4): how long the runner holds a
    /// step for a rewrite ride's answer. The runner checks it against the
    /// `ride_wait_max_ms` its Setup request carried, and treats a value
    /// above that cap as a failed Setup naming `ride_wait_ms`.
    ReadyWithRideWait {
        initial: CompactionMessage,
        cache_ttl: Option<cortexkit_role_compaction_provider::cache_ttl::CacheTtl>,
        ride_wait_ms: u64,
    },
    /// Fail the actual provider call without an answer, not a REFUSE answer.
    Unavailable,
    Refuse {
        code: RefuseCode,
        reason: String,
        provider_code: Option<String>,
    },
}

/// The session identity carried in the Setup request body. `project_root` is
/// not a Setup field; the provider route carries it separately.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionSetupIdentity {
    /// The caller-chosen harness from the Setup body.
    pub harness: String,
    /// The runner's session handle from the Setup body.
    pub session: String,
}

/// The actual bind identity of the runner-to-provider route that delivered
/// Setup. Its harness may be generic and its session may be empty on a shared
/// project route; adapters observe these fields at provider ingress.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionRouteIdentity {
    /// The bind harness on the route that delivered Setup.
    pub harness: String,
    /// The bind session, empty for a shared project route.
    pub session: String,
    /// The project root on the incoming route, not carried in Setup.
    pub project_root: String,
}

/// A Setup call captured by the scripted provider. The adapter records the
/// request body and actual incoming route independently; it never fabricates
/// either identity from the configured Setup answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompactionCall {
    pub request_id: String,
    /// The `lineage_id` the Setup request body carried, absent when it
    /// carried none.
    pub lineage_id: Option<String>,
    /// The `harness` and `session` that arrived in the Setup request body.
    pub setup_identity: CompactionSetupIdentity,
    /// The runner-to-provider route's actual bind identity.
    pub route_identity: CompactionRouteIdentity,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CompactionObservation {
    /// Actual handoffs to the host's standalone implementation, observed
    /// outside the runner, not inferred from a completed run.
    pub standalone_handoffs: usize,
    /// Every Setup call the scripted provider received, in order.
    pub calls: Vec<CompactionCall>,
    /// Canonical messages captured from requests received by the scripted
    /// model while its answers are held. The runner adapter must translate
    /// each request to ModelEntry, preserving its content and sources; it must
    /// not substitute `session.read` responses or configured replacements.
    pub model_inputs: Vec<Vec<crate::wire::read::ModelEntry>>,
    /// Every advance notice the runner published for the session, in order,
    /// as the header set the bus carried (CONTRACT §5 of
    /// compaction-provider). Captured at the bus, never rebuilt from runner
    /// state.
    pub notices: Vec<std::collections::BTreeMap<String, String>>,
}

/// One range of a [`ScriptedView`], in schema-neutral form: half-open over
/// ordinals of the view's lineage.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptedRange {
    /// Send the transcript's messages in `[from, to)` as written.
    Keep { from: u64, to: u64 },
    /// Send `replacement`, one message per string, in place of `[from,
    /// to)`; `from == to` inserts before `to`. The adapter translates each
    /// string to a message in the runner's schema that holds it.
    Replace {
        from: u64,
        to: u64,
        replacement: Vec<String>,
    },
}

/// A view the scripted compaction provider publishes
/// ([`crate::LlmRunnerSubject::publish_compaction_view`]), in schema-neutral
/// form. The adapter computes the digest and assigns the version.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScriptedView {
    /// The lineage whose ordinals the ranges use.
    pub lineage_id: String,
    /// The ordinal and `mid` of the last message the view covers.
    pub covers_through: u64,
    pub covers_through_mid: String,
    pub ranges: Vec<ScriptedRange>,
    /// The view's cut: the last ordinal its replacements stand for. Its
    /// `cut_id` is the adapter's to choose.
    pub cut: Option<u64>,
}

/// How the scripted compaction provider answers the next rewrite ride of a
/// session (compaction-provider §8: cause `ttl_expired`, `model_change`,
/// `account_change` or `surface_change`): after `answer_after_ms` milliseconds from the ride's
/// arrival, with `view` or, when it is `None`, `nothing`. A delay past the
/// runner's deadline for the ride means the runner never sees the answer:
/// the ride times out. A view is
/// published and translated as [`LlmRunnerSubject::publish_compaction_view`]
/// does, so it can apply at the ride's step.
///
/// Non-exhaustive: build one with [`ScriptedRide::view_after`] or
/// [`ScriptedRide::nothing_after`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScriptedRide {
    pub answer_after_ms: u64,
    pub view: Option<ScriptedView>,
}

impl ScriptedRide {
    /// Answer with `view` after `answer_after_ms` milliseconds; a runner
    /// whose ride deadline is shorter never sees it.
    pub fn view_after(answer_after_ms: u64, view: ScriptedView) -> Self {
        Self {
            answer_after_ms,
            view: Some(view),
        }
    }

    /// Answer `nothing` after `answer_after_ms` milliseconds; a runner whose
    /// ride deadline is shorter times the ride out first.
    pub fn nothing_after(answer_after_ms: u64) -> Self {
        Self {
            answer_after_ms,
            view: None,
        }
    }
}

pub(crate) fn initial(marker: &str) -> CompactionMessage {
    CompactionMessage {
        compaction_id: marker.into(),
        version: 1,
        from: 0,
        to: 0,
        replacement: vec![format!("{marker}-head"), format!("{marker}-note")],
    }
}

pub(crate) fn text(value: &str) -> ScriptedTurn {
    ScriptedTurn::new(vec![ScriptedPart::Text(value.into())])
}

pub(crate) async fn observation<S: LlmRunnerSubject>(
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

pub(crate) async fn install<S>(
    case: &Case<'_, S>,
    label: &str,
    setup: CompactionSetup,
    model: Script,
) -> Result<Session<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open(label, model).await?;
    case.subject
        .install_compaction_setup(&session.name, setup)
        .await
        .map_err(|e| e.to_string())?;
    Ok(session)
}

pub(crate) async fn send<S>(
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

pub(crate) async fn release_model<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
) -> Result<(), String> {
    subject
        .release_compaction_model(session)
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

/// The result of the run a send started, once that run has ended. A run that
/// has not ended yet answers its current state (`active`), so this polls.
pub(crate) async fn result<S>(
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
        "compaction_setup_route_session_matches_setup" => route_session_matches_setup(case).await,
        "session_ref_uses_runner_identity" => session_ref_uses_runner_identity(case).await,
        _ => Err(format!("unknown compaction case {name}")),
    }
}

async fn session_ref_uses_runner_identity<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = install(
        case,
        "session-reference",
        CompactionSetup::Ready(initial("session-reference")),
        Script {
            turns: vec![text("session-reference-answer")],
        },
    )
    .await?;
    send(case, &mut session, "session-reference-prompt").await?;
    release_model(case.subject, &session.name).await?;
    let observed = observation(case.subject, &session.name).await?;
    let setup = &observed
        .calls
        .first()
        .ok_or("no observed Setup call for the session reference")?
        .setup_identity;
    let expected = crate::wire::session_ref(&crate::wire::SessionIdentity {
        runner: case.subject.runner_id(),
        harness: &setup.harness,
        session: &setup.session,
    });
    if observed.notices.is_empty() {
        return Err("no advance notice observed for the session reference".into());
    }
    for notice in &observed.notices {
        if notice.get("session_ref") != Some(&expected) {
            return Err(format!(
                "session_ref {:?} differs from shared runner/harness/session reference {expected}",
                notice.get("session_ref")
            ));
        }
    }
    Ok(())
}

async fn route_session_matches_setup<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = install(
        case,
        "route-identity",
        CompactionSetup::Ready(initial("route-identity")),
        Script {
            turns: vec![text("route-identity-answer")],
        },
    )
    .await?;
    send(case, &mut session, "route-identity-prompt").await?;
    release_model(case.subject, &session.name).await?;

    let observation = observation(case.subject, &session.name).await?;
    if observation.calls.len() != 1 {
        return Err(format!(
            "expected one observed Setup call, got {}",
            observation.calls.len()
        ));
    }
    let call = &observation.calls[0];
    let setup_session = &call.setup_identity.session;
    let route_session = &call.route_identity.session;
    if !route_session.is_empty() && route_session != setup_session {
        return Err(format!(
            "single-session provider route binds session {route_session:?}, but Setup reports {setup_session:?}"
        ));
    }
    // A provider that finds the route bound to another session refuses Setup
    // `setup_identity_mismatch`: the run ends `error` under that code, with
    // no model request. The run's error is read from `run.result`, so this
    // half needs `run_ops`.
    if !case.declared.contains(&Capability::RunOps) {
        return Ok(());
    }
    let reason = "the route is bound to another session";
    let mut refused = install(
        case,
        "route-identity-refused",
        CompactionSetup::Refuse {
            code: RefuseCode::SetupIdentityMismatch,
            reason: reason.into(),
            provider_code: None,
        },
        Script {
            turns: vec![text("route-identity-refused-answer")],
        },
    )
    .await?;
    let reply = send(case, &mut refused, "route-identity-refused-prompt").await?;
    let ended = result(case, &refused, &reply).await?;
    if ended.run_state() != RunState::Error {
        return Err(format!(
            "a Setup refused setup_identity_mismatch ended {:?}, not error",
            ended.state
        ));
    }
    let error = ended
        .error
        .ok_or("a Setup refused setup_identity_mismatch has no run error")?;
    refusal_error(
        &error,
        RefuseCode::SetupIdentityMismatch.as_str(),
        reason,
        None,
    )
    .map_err(|e| format!("a Setup refused setup_identity_mismatch: {e}"))?;
    if !self::observation(case.subject, &refused.name)
        .await?
        .model_inputs
        .is_empty()
    {
        return Err("a Setup refused setup_identity_mismatch sent a model request".into());
    }
    Ok(())
}

async fn ranges<S>(case: &Case<'_, S>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let head = initial(&case.mint.next("ranges"));
    let mut session = install(
        case,
        "ranges",
        CompactionSetup::Ready(head.clone()),
        Script {
            turns: vec![text("first-answer")],
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
    let obs = observation(case.subject, &session.name).await?;
    match obs.model_inputs.first() {
        Some(input) if *input == model_page(&session.route, 0, false).await?.messages[..3] => {}
        other => {
            return Err(format!(
                "the held model request does not carry the canonical view with Setup's head: \
                 {other:?}"
            ))
        }
    }
    Ok(())
}

/// Check `RunResult.error`'s refusal fields against the configured role code,
/// reason and finer code. Required class/message are checked by wire decoding.
fn refusal_error(
    error: &cortexkit_role_llm_runner::run::RunError,
    code: &str,
    reason: &str,
    finer: Option<&str>,
) -> Result<(), String> {
    if error.provider_code != code
        || error.reason.as_deref() != Some(reason)
        || error.provider.as_deref().is_none_or(str::is_empty)
        || error.provider_detail_code.as_deref() != finer
    {
        return Err(format!(
            "REFUSE mapping lost role code/provider/reason or finer diagnostic: {error:?}"
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
        // A host with no standalone Magic Context fails the run by name; a
        // host running it in-process may hand the session to it instead.
        CompactionSetup::Refuse {
            code: RefuseCode::StandaloneSessionNotMigrated,
            reason: "run doctor to move this session".into(),
            provider_code: None,
        },
    ]
    .into_iter()
    .filter(|setup| {
        !(case.subject.standalone_in_process()
            && matches!(
                setup,
                CompactionSetup::Refuse {
                    code: RefuseCode::StandaloneSessionNotMigrated,
                    ..
                }
            ))
    }) {
        let mut session = install(
            case,
            "refusals",
            setup.clone(),
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
                if error.provider_code != "compaction_unavailable"
                    || error.provider_detail_code.is_some()
                {
                    return Err(format!(
                        "unavailable was mapped to a provider refusal: {error:?}"
                    ));
                }
            }
            CompactionSetup::Refuse {
                code,
                reason,
                provider_code,
            } => refusal_error(&error, code.as_str(), reason, provider_code.as_deref())?,
            CompactionSetup::Ready(_)
            | CompactionSetup::ReadyWithCheckpoint { .. }
            | CompactionSetup::ReadyWithCacheTtl { .. }
            | CompactionSetup::ReadyWithRideWait { .. } => {
                unreachable!()
            }
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
        case.subject
            .install_compaction_setup(
                &session.name,
                CompactionSetup::Ready(initial(&case.mint.next("retry-head"))),
            )
            .await
            .map_err(|e| e.to_string())?;
        send(case, &mut session, "retry-after-refusal").await?;
        release_model(case.subject, &session.name).await?;
        let obs = observation(case.subject, &session.name).await?;
        if obs.calls.len() != 2 || obs.model_inputs.len() != 1 {
            return Err(
                "session unusable or Setup not retried after failed/refused initial answer".into(),
            );
        }
    }
    Ok(())
}

/// The Setup durability check, in its own persistent state directory, with
/// its kill recorded in the conformance driver's ledger: kill at
/// `FoldRecorded`, when Setup's initial CompactionMessage is durable and the
/// first model request not yet sent.
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
    let setup = CompactionSetup::Ready(head.clone());
    let model = Script {
        turns: vec![text("setup-final"), text("followup-final")],
    };
    let handle = driver.spawn(&root).await.map_err(|e| e.to_string())?;
    let case = Case {
        subject,
        handle: &handle,
        declared,
        mint,
    };
    let mut live = install(&case, "compaction-control", setup.clone(), model.clone()).await?;
    send(&case, &mut live, "same-prompt").await?;
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
    let cut = install(&case, "compaction-cut", setup, model).await?;
    let params = send_params(
        subject,
        &cut.name,
        true,
        "same-prompt",
        &mint.next("send"),
        None,
    );
    let cut_name = cut.name.clone();
    let route = &cut.route;
    let trigger: Trigger<'_> = Box::pin(async move {
        let _ = route.request(ops::SESSION_SEND, params).await;
    });
    drop(live);
    driver
        .kill_at(handle, &KillPoint::new(points::FOLD_RECORDED), trigger)
        .await
        .map_err(|e| e.to_string())?;
    drop(cut);
    let handle = driver.restart(&root).await.map_err(|e| e.to_string())?;
    // Reopen cut_name after restart so the provider retains the session's
    // captured calls and configured answer under the original identity.
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
    if obs.calls.len() != 1 || obs.model_inputs.len() != 2 {
        return Err("Setup re-ran or stopped being usable on a later send".into());
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
    finish_crash(subject, handle, session, head, expected).await?;
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
    if obs.calls.len() != 1 {
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
    if obs.calls.len() != 1 {
        return Err("Setup was re-run after its durable answer".into());
    }
    Ok(())
}

/// A standalone refusal is a handoff only before the runner ever serves the
/// session and only on a host with standalone in-process.
pub async fn standalone_refusal<S>(case: &Case<'_, S>) -> Result<crate::cases::Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    use crate::cases::Ending;
    let refusal = CompactionSetup::Refuse {
        code: RefuseCode::StandaloneSessionNotMigrated,
        reason: "move required".into(),
        provider_code: None,
    };
    let mut first = install(
        case,
        "standalone-first",
        refusal.clone(),
        Script {
            turns: vec![text("standalone answer")],
        },
    )
    .await?;
    let reply = send(case, &mut first, "first prompt").await?;
    let ended = result(case, &first, &reply).await?;
    let obs = observation(case.subject, &first.name).await?;
    if case.subject.standalone_in_process() {
        if obs.standalone_handoffs != 1
            || ended.run_state() != RunState::Completed
            || !obs.model_inputs.is_empty()
        {
            return Err("first standalone refusal must hand off in the same step, not fail or forward unmanaged history through the runner".into());
        }
    } else if ended.run_state() != RunState::Error
        || ended
            .error
            .as_ref()
            .is_none_or(|e| e.provider_code != "standalone_session_not_migrated")
        || !obs.model_inputs.is_empty()
        || obs.standalone_handoffs != 0
    {
        return Err("a host without standalone must fail standalone_session_not_migrated, never forward unmanaged history".into());
    }
    let mut served = install(
        case,
        "standalone-served",
        CompactionSetup::Ready(initial("served-head")),
        Script {
            turns: vec![text("managed answer"), text("must not reach model")],
        },
    )
    .await?;
    send(case, &mut served, "managed prompt").await?;
    release_model(case.subject, &served.name).await?;
    if let Err(e) = case
        .subject
        .require_setup_again(case.handle.inner(), &served.name)
        .await
    {
        return Ok(Ending::PassedWithInapplicableRule(format!(
            "late refusal: the adapter cannot require Setup again: {e}"
        )));
    }
    case.subject
        .install_compaction_setup(&served.name, refusal)
        .await
        .map_err(|e| e.to_string())?;
    let reply = send(case, &mut served, "late refusal").await?;
    let ended = result(case, &served, &reply).await?;
    let obs = observation(case.subject, &served.name).await?;
    if ended.run_state() != RunState::Error
        || ended
            .error
            .as_ref()
            .is_none_or(|e| e.provider_code != "standalone_session_not_migrated")
        || obs.standalone_handoffs != 0
        || obs.model_inputs.len() != 1
    {
        return Err("a runner that has served the session must fail the later standalone_session_not_migrated by name, never hand it back".into());
    }
    Ok(Ending::Passed)
}
