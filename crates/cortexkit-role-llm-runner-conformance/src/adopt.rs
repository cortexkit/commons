//! `session.adopt` cases, run only when the subject declares `adopt`.
//!
//! Each case starts its session the way a person's app does
//! ([`LlmRunnerSubject::start_host_session`]), so the runner applied a plan
//! at host start and no role request has reached the session. Every case
//! reads the transcript before and after each adoption: an adoption starts no
//! run, writes no message, and leaves a run in progress running.

use cortexkit_role_harness::RouteStamp;
use cortexkit_role_llm_runner::{
    adopt::{AdoptReply, AdoptRequest, PLAN_FIELD},
    baseline::{states as baseline_states, BaselineReply, BaselineRequest},
    capabilities as groups,
    describe::RoleDescribe,
    errors::{self, PlanDriftDetail},
    ops,
    read::{HeadMeta, ReadMessage},
    run::{states as run_states, RunState, RunStatus},
    send::SendReply,
    subscribe::{SubscribeFrom, SubscribeRequest},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, head, holding, read_all,
        run_result, send_params, wait_run_end, MAX_POLLS,
    },
    route::{Reply, RunnerRoute, SubscribeOutcome},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

type Check = Result<(), String>;

/// What the transcript holds: the head metadata and every message.
type Transcript = (HeadMeta, Vec<ReadMessage>);

async fn transcript<R: RunnerRoute>(route: &R) -> Result<Transcript, String> {
    Ok((head(route).await?, read_all(route).await?.1))
}

/// Fail unless the transcript reads as `before`: the same lineage, last
/// ordinal and last run in the same state, and the same messages. The
/// head's stream position and update time are not compared: an adoption is a
/// durable record of its own, which a runner may count there.
async fn unchanged<R: RunnerRoute>(route: &R, before: &Transcript, what: &str) -> Check {
    let after = transcript(route).await?;
    let key = |(head, messages): &Transcript| {
        (
            head.lineage_id.clone(),
            head.last_ordinal,
            head.last_run_state.clone(),
            messages.clone(),
        )
    };
    if key(&after) != key(before) {
        return Err(format!(
            "{what} changed the transcript: before {:?} with {} messages, after {:?} with {} \
             messages",
            before.0,
            before.1.len(),
            after.0,
            after.1.len()
        ));
    }
    Ok(())
}

fn request(send_id: &str, plan: &Map<String, Value>) -> Value {
    serde_json::to_value(AdoptRequest::new(send_id, plan.clone())).expect("serializes")
}

async fn adopt_raw<R: RunnerRoute>(
    route: &R,
    send_id: &str,
    plan: &Map<String, Value>,
) -> Result<Reply, String> {
    call(route, ops::SESSION_ADOPT, request(send_id, plan)).await
}

/// An adoption the case expects to succeed, with its reply checked for the
/// role's fixed member.
async fn adopt<R: RunnerRoute>(
    route: &R,
    send_id: &str,
    plan: &Map<String, Value>,
) -> Result<AdoptReply, String> {
    let reply: AdoptReply = decode(
        "session.adopt",
        expect_ok("session.adopt", adopt_raw(route, send_id, plan).await?)?,
    )?;
    if !reply.owner_bound || reply.applied_plan_digest.is_empty() {
        return Err(format!(
            "session.adopt answered {reply:?}: owner_bound must be true and \
             applied_plan_digest must name the plan"
        ));
    }
    Ok(reply)
}

async fn baseline<R: RunnerRoute>(route: &R) -> Result<Reply, String> {
    let params = serde_json::to_value(BaselineRequest::default()).expect("serializes");
    call(route, ops::SESSION_BASELINE, params).await
}

/// Wait until the session has no run in progress, so a person's first turn
/// has ended before the case reads the transcript it compares against.
async fn idle<S>(case: &Case<'_, S>, route: &S::Route) -> Result<HeadMeta, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    for _ in 0..MAX_POLLS {
        let head = head(route).await?;
        if head
            .last_run_state
            .as_ref()
            .is_none_or(|last| RunState::parse(&last.state).is_terminal())
        {
            return Ok(head);
        }
        case.subject.pause().await;
    }
    Err(format!(
        "the person's turn did not end within {MAX_POLLS} polls"
    ))
}

/// The suite sends `session.adopt` only to a runner whose live
/// `role.describe` declares `adopt`, as a consumer must; the subject's
/// declaration alone is not enough.
async fn require_declared<R: RunnerRoute>(route: &R) -> Check {
    let describe: RoleDescribe = decode(
        "role.describe",
        expect_ok(
            "role.describe",
            call(route, ops::ROLE_DESCRIBE, serde_json::json!({})).await?,
        )?,
    )?;
    if describe.declares(groups::ADOPT) {
        Ok(())
    } else {
        Err("role.describe does not declare adopt, so the suite does not send session.adopt".into())
    }
}

/// A session the host started with its plan applied, and with one finished
/// person's turn when `prompt` is set. Returns the session, with the owner's
/// route, and the plan the host applied.
async fn host_session<S>(
    case: &Case<'_, S>,
    label: &str,
    script: Script,
    prompt: Option<&str>,
) -> Result<(Session<S::Route>, Map<String, Value>), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open(label, script).await?;
    require_declared(&session.route).await?;
    let plan = host_plan(case, &session.name, false)?;
    case.subject
        .start_host_session(case.handle.inner(), &session.name, &plan, prompt)
        .await
        .map_err(|e| format!("starting {} as the host would: {e}", session.name))?;
    Ok((session, plan))
}

fn host_plan<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &str,
    alternate: bool,
) -> Result<Map<String, Value>, String> {
    case.subject
        .host_plan(session, alternate)
        .ok_or_else(|| "the subject builds no host plan, which adopt requires".into())
}

/// A host session with one finished person's turn: a prompt and one answer.
async fn answered_session<S>(
    case: &Case<'_, S>,
    label: &str,
) -> Result<(Session<S::Route>, Map<String, Value>, Transcript), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (part, _) = case.mint.text("person-answer");
    let prompt = case.mint.next("person-prompt");
    let (session, plan) = host_session(
        case,
        label,
        Script {
            turns: vec![ScriptedTurn::new(vec![part])],
        },
        Some(&prompt),
    )
    .await?;
    idle(case, &session.route).await?;
    let before = transcript(&session.route).await?;
    if before.0.last_ordinal.is_none() {
        return Err("the person's turn wrote no message the suite can read".into());
    }
    Ok((session, plan, before))
}

fn same_reply(what: &str, first: &AdoptReply, again: &AdoptReply) -> Check {
    if first == again {
        Ok(())
    } else {
        Err(format!(
            "{what} answered {again:?}, not the original {first:?}"
        ))
    }
}

fn last_ordinal_matches(reply: &AdoptReply, head: &HeadMeta) -> Check {
    if reply.last_ordinal == head.last_ordinal {
        Ok(())
    } else {
        Err(format!(
            "session.adopt answered last_ordinal {:?}, but session.head's last_ordinal is {:?}",
            reply.last_ordinal, head.last_ordinal
        ))
    }
}

async fn stranger<S>(case: &Case<'_, S>, session: &str) -> Result<S::Route, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let stamp: RouteStamp = case.subject.stranger_stamp();
    case.subject
        .session_route(case.handle.inner(), session, &stamp)
        .await
        .map_err(|e| format!("opening {session} as a stranger: {e}"))
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if name == "adopt_host_turn_is_a_run" {
        return host_turn_is_a_run(case).await;
    }
    let check = match name {
        "adopt_binds_owner_without_a_run" => binds_owner(case).await,
        "adopt_exact_repeat_replays_reply" => exact_repeat(case).await,
        "adopt_send_id_reuse_names_plan" => send_id_reuse(case).await,
        "adopt_plan_drift_refused" => plan_drift(case).await,
        "adopt_during_active_run" => during_active_run(case).await,
        "adopt_other_owner_refused" => other_owner(case).await,
        "adopt_same_owner_readopts" => same_owner_readopts(case).await,
        "adopt_then_owner_sends" => then_owner_sends(case).await,
        other => Err(format!("unknown adopt case {other}")),
    };
    check.map(|()| Ending::Passed)
}

/// A turn the person starts in their own app is a run like any other: it
/// has a `run_id` on `session.head`, `run.result` answers it, and
/// `run.status` and `session.subscribe` show it where declared. It carries
/// no `send_id`, which is how the owner tells its own runs from it.
/// Inapplicable when the adapter cannot start a host turn.
async fn host_turn_is_a_run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (first, answer) = case.mint.text("host-turn-answer");
    let (second, _) = case.mint.text("host-turn-owner-answer");
    let prompt = case.mint.next("host-turn-prompt");
    let session = case
        .open(
            "adopt-host-turn",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![first]),
                    ScriptedTurn::new(vec![second]),
                ],
            },
        )
        .await?;
    require_declared(&session.route).await?;
    let plan = host_plan(case, &session.name, false)?;
    if let Err(error) = case
        .subject
        .start_host_session(case.handle.inner(), &session.name, &plan, Some(&prompt))
        .await
    {
        return Ok(Ending::Inapplicable(format!(
            "the adapter cannot start a host turn: {error}"
        )));
    }
    let head = idle(case, &session.route).await?;
    let last = head.last_run_state.ok_or(
        "the person's turn left no run on session.head: a turn the host starts is a run, with \
         a run_id",
    )?;
    let what = format!("run.result of the person's run {}", last.run_id);
    let result = run_result(&session.route, &last.run_id)
        .await
        .map_err(|e| format!("{what}: {e}"))?;
    let text = result.final_message.as_ref().map(|m| m.text.clone());
    if !result.run_state().is_terminal() || !text.as_deref().is_some_and(|t| t.contains(&answer)) {
        return Err(format!(
            "{what} answered state {} with final text {text:?}, not the person's finished turn",
            result.state
        ));
    }
    if case.declared.contains(&Capability::RunStatus) {
        let status: RunStatus = decode(
            "run.status",
            expect_ok(
                "run.status of the person's run",
                call(
                    &session.route,
                    ops::RUN_STATUS,
                    json!({ "run_id": last.run_id }),
                )
                .await?,
            )?,
        )?;
        if status.state != result.state || status.directives != result.directives {
            return Err(format!(
                "run.status of the person's run answered {status:?}, which disagrees with \
                 run.result {result:?}"
            ));
        }
    }
    if case.declared.contains(&Capability::Streaming) {
        let params =
            serde_json::to_value(SubscribeRequest::new(SubscribeFrom::Start)).expect("serializes");
        match session
            .route
            .subscribe(params)
            .await
            .map_err(|e| format!("session.subscribe: the route failed: {}", e.message))?
        {
            SubscribeOutcome::Refused(error) => {
                return Err(format!(
                    "session.subscribe of the person's session was refused {}",
                    error.code
                ))
            }
            SubscribeOutcome::Events(events) if events.is_empty() => {
                return Err(
                    "session.subscribe from the start shows nothing of the person's run".into(),
                )
            }
            SubscribeOutcome::Events(_) => {}
        }
    }
    adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    let owner_prompt = case.mint.next("host-turn-owner-prompt");
    let reply: SendReply = decode(
        "the owner's session.send after adoption",
        expect_ok(
            "the owner's session.send after adoption",
            call(
                &session.route,
                ops::SESSION_SEND,
                send_params(
                    case.subject,
                    &session.name,
                    false,
                    &owner_prompt,
                    &case.mint.next("send"),
                    None,
                ),
            )
            .await?,
        )?,
    )?;
    let end = wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        Some(&last.run_id),
        &reply,
    )
    .await?;
    if end.run_id == last.run_id {
        return Err(format!(
            "the owner's send ran as {}, the person's run: each turn is a run of its own",
            end.run_id
        ));
    }
    Ok(Ending::Passed)
}

async fn binds_owner<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    // A session the host started with no message yet.
    let (empty, plan) = host_session(case, "adopt-empty", Script::default(), None).await?;
    let before = transcript(&empty.route).await?;
    let reply = adopt(&empty.route, &case.mint.next("adopt"), &plan).await?;
    last_ordinal_matches(&reply, &before.0)?;
    unchanged(&empty.route, &before, "adopting a session with no message").await?;

    // A session after the person's first turn.
    let (session, plan, before) = answered_session(case, "adopt-first").await?;
    let reply = adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    last_ordinal_matches(&reply, &before.0)?;
    unchanged(&session.route, &before, "session.adopt").await?;
    let owner: BaselineReply = decode(
        "session.baseline",
        expect_ok(
            "the adopting owner's session.baseline",
            baseline(&session.route).await?,
        )?,
    )?;
    if owner.state != baseline_states::READY || owner.baseline.is_none() {
        return Err(format!(
            "after session.adopt, the owner's session.baseline answered {owner:?}, not ready \
             with a baseline"
        ));
    }
    let stranger = stranger(case, &session.name).await?;
    let error = expect_error(
        "a stranger's session.baseline after adoption",
        baseline(&stranger).await?,
    )?;
    expect_code(
        "a stranger's session.baseline after adoption",
        &error,
        errors::SCOPE_OWNER_MISMATCH,
    )
}

async fn exact_repeat<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, plan, before) = answered_session(case, "adopt-repeat").await?;
    let send_id = case.mint.next("adopt");
    let first = adopt(&session.route, &send_id, &plan).await?;
    let again = adopt(&session.route, &send_id, &plan).await?;
    same_reply("an exact repeat of session.adopt", &first, &again)?;
    unchanged(&session.route, &before, "an adoption and its repeat").await
}

async fn send_id_reuse<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, plan, before) = answered_session(case, "adopt-reuse").await?;
    let other = host_plan(case, &session.name, true)?;
    if other == plan {
        return Err("the subject's alternate host plan equals its host plan".into());
    }
    let send_id = case.mint.next("adopt");
    let first = adopt(&session.route, &send_id, &plan).await?;
    let error = expect_error(
        "session.adopt reusing a send_id with another plan",
        adopt_raw(&session.route, &send_id, &other).await?,
    )?;
    expect_field(
        "session.adopt reusing a send_id with another plan",
        &error,
        errors::SEND_ID_REUSE,
        PLAN_FIELD,
    )?;
    let again = adopt(&session.route, &send_id, &plan).await?;
    same_reply(
        "an exact repeat after a send_id_reuse refusal",
        &first,
        &again,
    )?;
    unchanged(&session.route, &before, "a refused send_id reuse").await
}

async fn plan_drift<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, plan, before) = answered_session(case, "adopt-drift").await?;
    let other = host_plan(case, &session.name, true)?;
    if other == plan {
        return Err("the subject's alternate host plan equals its host plan".into());
    }
    // Another caller sends the drifting plan: had the refusal bound it as
    // the owner, the owner's adoption below would be refused.
    let stranger = stranger(case, &session.name).await?;
    let send_id = case.mint.next("adopt");
    let error = expect_error(
        "session.adopt with a plan other than the host's",
        adopt_raw(&stranger, &send_id, &other).await?,
    )?;
    expect_code(
        "session.adopt with a plan other than the host's",
        &error,
        errors::PLAN_DRIFT,
    )?;
    let detail: PlanDriftDetail = decode(
        "the plan_drift detail",
        error.detail.clone().unwrap_or(Value::Null),
    )?;
    let frozen = detail
        .frozen
        .clone()
        .ok_or("the plan_drift detail of a session the host started with a plan has no frozen")?;
    if frozen == detail.sent {
        return Err(format!(
            "the plan_drift detail names the same identity twice: {detail:?}"
        ));
    }
    for plan in [&plan, &other] {
        let content = Value::Object(plan.clone()).to_string();
        if content == frozen || content == detail.sent {
            return Err("the plan_drift detail carries plan content, not identities".into());
        }
    }
    unchanged(&session.route, &before, "a plan_drift refusal").await?;
    // The refusal recorded nothing, its send_id and its caller included:
    // the owner adopts with the host's plan under the same send_id, bound to
    // the identity the refusal named.
    let reply = adopt(&session.route, &send_id, &plan).await?;
    if reply.applied_plan_digest != frozen {
        return Err(format!(
            "plan_drift named frozen {frozen}, but adopting the host's plan answered \
             applied_plan_digest {}",
            reply.applied_plan_digest
        ));
    }
    unchanged(&session.route, &before, "session.adopt after a refusal").await?;

    // A session the host never started has no applied plan to match.
    let never = case.open("adopt-never-started", Script::default()).await?;
    let error = expect_error(
        "session.adopt of a session with no applied plan",
        adopt_raw(&never.route, &case.mint.next("adopt"), &plan).await?,
    )?;
    expect_code(
        "session.adopt of a session with no applied plan",
        &error,
        errors::PLAN_DRIFT,
    )?;
    let detail: PlanDriftDetail =
        decode("the plan_drift detail", error.detail.unwrap_or(Value::Null))?;
    if detail.frozen.is_some() {
        return Err(format!(
            "a session with no applied plan answered plan_drift naming frozen: {detail:?}"
        ));
    }
    Ok(())
}

async fn during_active_run<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut held = case.mint.call("adopt-held");
    held.hold = true;
    let (answer, marker) = case.mint.text("person-answer");
    let prompt = case.mint.next("person-prompt");
    let (session, plan) = host_session(
        case,
        "adopt-active",
        Script {
            turns: vec![
                ScriptedTurn::new(vec![ScriptedPart::ToolCall(held.clone())]),
                ScriptedTurn::new(vec![answer]),
            ],
        },
        Some(&prompt),
    )
    .await?;
    case.subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("waiting for the person's held tool call: {e}"))?;
    let before = transcript(&session.route).await?;
    let running = before
        .0
        .last_run_state
        .clone()
        .filter(|last| !RunState::parse(&last.state).is_terminal())
        .ok_or_else(|| {
            format!(
                "the person's turn is not in progress while its tool call is held: {:?}",
                before.0
            )
        })?;
    let reply = adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    last_ordinal_matches(&reply, &before.0)?;
    unchanged(&session.route, &before, "session.adopt during a run").await?;
    case.subject
        .release_tool_call(&held.arguments)
        .await
        .map_err(|e| format!("releasing the person's held tool call: {e}"))?;
    let after = idle(case, &session.route).await?;
    match after.last_run_state {
        Some(last) if last.run_id == running.run_id && last.state == run_states::COMPLETED => {}
        other => {
            return Err(format!(
                "the person's run {} did not run on to completed after the adoption: the last \
                 run is {other:?}",
                running.run_id
            ))
        }
    }
    if case.subject.tool_invocations(&held.arguments).await != 1 {
        return Err("the held tool call did not run exactly once".into());
    }
    let (_, messages) = read_all(&session.route).await?;
    if holding(&messages, &marker) != 1 {
        return Err("the person's run did not write its answer exactly once".into());
    }
    Ok(())
}

async fn other_owner<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, plan, before) = answered_session(case, "adopt-owner").await?;
    let send_id = case.mint.next("adopt");
    let first = adopt(&session.route, &send_id, &plan).await?;
    let stranger = stranger(case, &session.name).await?;
    for (what, id) in [
        ("another caller's session.adopt", case.mint.next("adopt")),
        (
            "another caller repeating the owner's session.adopt",
            send_id.clone(),
        ),
    ] {
        let error = expect_error(what, adopt_raw(&stranger, &id, &plan).await?)?;
        expect_code(what, &error, errors::SCOPE_OWNER_MISMATCH)?;
    }
    expect_ok(
        "the owner's session.baseline after another caller's adoption",
        baseline(&session.route).await?,
    )?;
    let error = expect_error(
        "the other caller's session.baseline",
        baseline(&stranger).await?,
    )?;
    expect_code(
        "the other caller's session.baseline",
        &error,
        errors::SCOPE_OWNER_MISMATCH,
    )?;
    let again = adopt(&session.route, &send_id, &plan).await?;
    same_reply(
        "the owner's repeat after a refused takeover",
        &first,
        &again,
    )?;
    unchanged(&session.route, &before, "a refused takeover").await
}

async fn same_owner_readopts<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, plan, before) = answered_session(case, "adopt-again").await?;
    let first = adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    unchanged(&session.route, &before, "the first adoption").await?;
    let second = adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    if second.applied_plan_digest != first.applied_plan_digest {
        return Err(format!(
            "re-adopting with the same plan answered applied_plan_digest {}, not {}",
            second.applied_plan_digest, first.applied_plan_digest
        ));
    }
    last_ordinal_matches(&second, &before.0)?;
    unchanged(&session.route, &before, "a second adoption").await
}

async fn then_owner_sends<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (first, _) = case.mint.text("person-answer");
    let (second, answer) = case.mint.text("owner-answer");
    let person = case.mint.next("person-prompt");
    let (session, plan) = host_session(
        case,
        "adopt-send",
        Script {
            turns: vec![
                ScriptedTurn::new(vec![first]),
                ScriptedTurn::new(vec![second]),
            ],
        },
        Some(&person),
    )
    .await?;
    let before = idle(case, &session.route).await?;
    adopt(&session.route, &case.mint.next("adopt"), &plan).await?;
    let stranger = stranger(case, &session.name).await?;
    let refused = case.mint.next("stranger-prompt");
    let error = expect_error(
        "another caller's send after adoption",
        call(
            &stranger,
            ops::SESSION_SEND,
            send_params(
                case.subject,
                &session.name,
                false,
                &refused,
                &case.mint.next("send"),
                None,
            ),
        )
        .await?,
    )?;
    expect_code(
        "another caller's send after adoption",
        &error,
        errors::SCOPE_OWNER_MISMATCH,
    )?;
    let prompt = case.mint.next("owner-prompt");
    let reply: SendReply = decode(
        "the owner's session.send after adoption",
        expect_ok(
            "the owner's session.send after adoption",
            call(
                &session.route,
                ops::SESSION_SEND,
                send_params(
                    case.subject,
                    &session.name,
                    false,
                    &prompt,
                    &case.mint.next("send"),
                    None,
                ),
            )
            .await?,
        )?,
    )?;
    let before_run = before.last_run_state.map(|last| last.run_id);
    wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before_run.as_deref(),
        &reply,
    )
    .await?;
    let (_, messages) = read_all(&session.route).await?;
    for (marker, count) in [(&prompt, 1), (&answer, 1), (&refused, 0)] {
        if holding(&messages, marker) != count {
            return Err(format!(
                "after adoption the transcript holds {marker} {} times, expected {count}",
                holding(&messages, marker)
            ));
        }
    }
    Ok(())
}
