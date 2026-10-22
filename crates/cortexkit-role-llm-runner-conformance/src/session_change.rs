//! The `session_change` group's cases (§10.2): `session.refresh` answers,
//! the next-run rule, identical repeats, stale generations, the host's
//! fixed tool set and `session.flush_prefix`. Tool sets are observed at the
//! scripted model, outside the runner; everything else over the route.

use std::collections::BTreeSet;

use cortexkit_role_llm_runner::{
    baseline::{Baseline, BaselineReply, BaselineRequest, PendingChange},
    errors, ops, rungs,
    send::SendReply,
    session_change::{
        FlushPrefixReply, FlushPrefixRequest, GenerationStaleDetail, RefreshReply, RefreshRequest,
        ToolOutsideHostSetDetail,
    },
};
use serde_json::{Map, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_code, expect_error, expect_ok, wait_run_end},
    route::{Reply, RunnerRoute},
    subject::{
        LlmRunnerSubject, RefreshPlan, RefreshVariant, Script, ScriptedPart, ScriptedToolCall,
        ScriptedTurn,
    },
};

type Tools = BTreeSet<String>;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "session_refresh_applies_from_next_run" => applies_from_next_run(case).await,
        "session_refresh_waits_for_its_rung" => waits_for_its_rung(case).await,
        "session_refresh_never_changes_a_running_run" => held(case, false).await,
        "flush_prefix_applies_pending" => held(case, true).await,
        "session_refresh_repeat_same_answer" => repeat(case).await,
        "session_refresh_generation_stale_refused" => stale(case).await,
        "session_refresh_tool_outside_host_set_refused" => outside_host_set(case).await,
        other => Err(format!("unknown session_change case {other}")),
    }
}

fn text(label: &str) -> ScriptedTurn {
    ScriptedTurn::new(vec![ScriptedPart::Text(label.into())])
}

/// Send a queued prompt into `session`. The session's first send carries
/// its plan.
async fn send<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    prompt: &str,
) -> Result<SendReply, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let send_id = case.mint.next("change-send");
    let params = case.params(session, prompt, &send_id, None);
    let reply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first.get_or_insert((send_id, prompt.to_owned()));
    Ok(reply)
}

/// Send a prompt and wait for the run it starts to end.
async fn run_once<S>(case: &Case<'_, S>, session: &mut Session<S::Route>) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let reply = send(case, session, "next run").await?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    Ok(())
}

async fn baseline<R: RunnerRoute>(route: &R) -> Result<Baseline, String> {
    let reply: BaselineReply = decode(
        "session.baseline",
        expect_ok(
            "session.baseline",
            call(
                route,
                ops::SESSION_BASELINE,
                serde_json::to_value(BaselineRequest::default()).unwrap(),
            )
            .await?,
        )?,
    )?;
    reply
        .baseline
        .ok_or_else(|| "session.baseline answered not_yet after the session's first send".into())
}

/// The tools-surface rungs the session states, cheapest first. A runner
/// declaring `session_change` states them in every ready baseline.
fn tool_rungs(baseline: &Baseline) -> Result<Vec<String>, String> {
    let rungs = baseline.rungs.as_ref().ok_or(
        "a runner declaring session_change stated no rungs in session.baseline, so no policy \
         can be chosen",
    )?;
    Ok(rungs::ALL
        .iter()
        .filter(|rung| rungs.tools.iter().any(|stated| stated == *rung))
        .map(|rung| (*rung).to_owned())
        .collect())
}

fn plan_for<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &str,
    variant: RefreshVariant,
) -> Option<RefreshPlan> {
    case.subject.refresh_plan(session, variant)
}

async fn refresh<R: RunnerRoute>(
    route: &R,
    plan: &Map<String, Value>,
    policy: &str,
    generation: u64,
) -> Result<Reply, String> {
    let request = RefreshRequest::new(plan.clone(), policy, generation);
    call(
        route,
        ops::SESSION_REFRESH,
        serde_json::to_value(request).unwrap(),
    )
    .await
}

async fn accepted<R: RunnerRoute>(
    route: &R,
    plan: &Map<String, Value>,
    policy: &str,
    generation: u64,
) -> Result<(RefreshReply, Value), String> {
    let body = expect_ok(
        "session.refresh",
        refresh(route, plan, policy, generation).await?,
    )?;
    let reply: RefreshReply = decode("session.refresh", body.clone())?;
    let pending = PendingChange {
        generation,
        policy: policy.to_owned(),
    };
    if reply.accepted_generation != generation || reply.pending != pending {
        return Err(format!(
            "session.refresh of generation {generation} with {policy} answered {body}, not \
             accepted_generation {generation} and pending {{generation: {generation}, policy: \
             {policy}}}"
        ));
    }
    Ok((reply, body))
}

/// The tool names of every model request of `session`, in order.
async fn requests<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &str,
) -> Result<Vec<Tools>, String> {
    Ok(case
        .subject
        .model_requests(session)
        .await
        .map_err(|e| format!("observing the model requests of {session}: {e}"))?
        .into_iter()
        .map(|request| request.tools.into_iter().collect())
        .collect())
}

/// The request at `index`, or a failure naming what was expected there.
fn request_at(seen: &[Tools], index: usize, what: &str) -> Result<Tools, String> {
    seen.get(index).cloned().ok_or_else(|| {
        format!(
            "expected a model request for {what} (request {index}), but the model saw only {} \
             requests",
            seen.len()
        )
    })
}

/// A request after the change applied offers every tool the new plan names,
/// and not the tool set of the first plan.
fn uses_new_plan(request: &Tools, before: &Tools, plan: &RefreshPlan) -> bool {
    request != before && plan.tools.iter().all(|tool| request.contains(tool))
}

/// Check the adapter's changed plan really changes what a request offers.
fn changed_plan(before: &Tools, plan: &RefreshPlan) -> Result<(), String> {
    let wanted: Tools = plan.tools.iter().cloned().collect();
    if &wanted == before {
        return Err(format!(
            "the adapter's ChangedTools refresh plan offers {wanted:?}, the tool set the session \
             already has, so no case could tell whether it applied"
        ));
    }
    Ok(())
}

/// An idle session; the `immediately` rung: the answer says the next run
/// uses the new plan with no run in progress, and it does.
async fn applies_from_next_run<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "change-next-run",
            Script {
                turns: vec![text("first run"), text("second run")],
            },
        )
        .await?;
    run_once(case, &mut session).await?;
    let before_change = baseline(&session.route).await?;
    if !tool_rungs(&before_change)?
        .iter()
        .any(|rung| rung == rungs::IMMEDIATELY)
    {
        return Ok(Ending::Inapplicable(
            "the session supports no immediately rung for tools".into(),
        ));
    }
    let Some(plan) = plan_for(case, &session.name, RefreshVariant::ChangedTools) else {
        return Ok(Ending::Inapplicable(
            "the adapter supplies no refresh plan with another tool set".into(),
        ));
    };
    let first = request_at(&requests(case, &session.name).await?, 0, "the first run")?;
    changed_plan(&first, &plan)?;
    let (reply, body) = accepted(&session.route, &plan.plan, rungs::IMMEDIATELY, 1).await?;
    if reply.run_in_progress || !reply.applies_from_next_run {
        return Err(format!(
            "a refresh with the immediately rung on an idle session answered {body}, not \
             run_in_progress: false and applies_from_next_run: true"
        ));
    }
    if baseline(&session.route).await?.accepted_generation != 1 {
        return Err("session.baseline does not show the accepted generation 1".into());
    }
    run_once(case, &mut session).await?;
    let next = request_at(&requests(case, &session.name).await?, 1, "the next run")?;
    if !uses_new_plan(&next, &first, &plan) {
        return Err(format!(
            "the answer said the next run uses the new plan, but its model request offered \
             {next:?} (the first run offered {first:?}; the new plan names {:?})",
            plan.tools
        ));
    }
    applied(&session.route, 1).await?;
    Ok(Ending::Passed)
}

/// `session.baseline` shows `generation` applied and nothing pending.
async fn applied<R: RunnerRoute>(route: &R, generation: u64) -> Result<(), String> {
    let after = baseline(route).await?;
    if after.applied_generation < generation
        || after.pending.is_some()
        || after
            .effective
            .as_ref()
            .is_none_or(|e| e.generation < generation)
    {
        return Err(format!(
            "after the run that used generation {generation}, session.baseline shows \
             applied_generation {}, pending {:?} and effective {:?}",
            after.applied_generation, after.pending, after.effective
        ));
    }
    Ok(())
}

/// A rung other than `immediately`: whichever way the answer says the next
/// run goes, the next run and `session.baseline` agree with it.
async fn waits_for_its_rung<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            "change-rung",
            Script {
                turns: vec![text("first run"), text("second run")],
            },
        )
        .await?;
    run_once(case, &mut session).await?;
    let stated = tool_rungs(&baseline(&session.route).await?)?;
    let Some(rung) = stated
        .iter()
        .find(|rung| *rung != rungs::IMMEDIATELY)
        .cloned()
    else {
        return Ok(Ending::Inapplicable(
            "the session supports no rung for tools other than immediately".into(),
        ));
    };
    let Some(plan) = plan_for(case, &session.name, RefreshVariant::ChangedTools) else {
        return Ok(Ending::Inapplicable(
            "the adapter supplies no refresh plan with another tool set".into(),
        ));
    };
    let first = request_at(&requests(case, &session.name).await?, 0, "the first run")?;
    changed_plan(&first, &plan)?;
    let (reply, body) = accepted(&session.route, &plan.plan, &rung, 1).await?;
    if reply.run_in_progress {
        return Err(format!(
            "a refresh on an idle session answered {body}, with run_in_progress: true"
        ));
    }
    run_once(case, &mut session).await?;
    let next = request_at(&requests(case, &session.name).await?, 1, "the next run")?;
    let after = baseline(&session.route).await?;
    if reply.applies_from_next_run {
        if !uses_new_plan(&next, &first, &plan) {
            return Err(format!(
                "the {rung} refresh answered applies_from_next_run: true, but the next run's \
                 model request offered {next:?}, not the new plan's tools {:?}",
                plan.tools
            ));
        }
        applied(&session.route, 1).await?;
    } else {
        let pending = PendingChange {
            generation: 1,
            policy: rung.clone(),
        };
        if next != first || after.applied_generation >= 1 || after.pending != Some(pending) {
            return Err(format!(
                "the {rung} refresh answered applies_from_next_run: false, but the next run's \
                 model request offered {next:?} (before: {first:?}) and session.baseline shows \
                 applied_generation {} and pending {:?}: the current plan must stay live until \
                 the change applies",
                after.applied_generation, after.pending
            ));
        }
    }
    Ok(Ending::Passed)
}

/// A refresh (`flush` false) or a flush (`flush` true) while a run is
/// running: the running run's later steps keep the tools it started with,
/// and the next run uses the new plan.
async fn held<S>(case: &Case<'_, S>, flush: bool) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut held_call: ScriptedToolCall = case.mint.call("change-held");
    held_call.hold = true;
    let mut session = case
        .open(
            if flush { "change-flush" } else { "change-held" },
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(held_call.clone())]),
                    text("the running run's last step"),
                    text("the next run"),
                ],
            },
        )
        .await?;
    let reply = send(case, &mut session, "a run held at its tool call").await?;
    case.subject
        .await_tool_call(&held_call.arguments)
        .await
        .map_err(|e| format!("waiting for the held call: {e}"))?;
    let stated = tool_rungs(&baseline(&session.route).await?)?;
    // The refresh case needs `immediately`, the rung that would apply
    // soonest. The flush case prefers a rung that would otherwise wait, so
    // only the flush can make it apply at the next run.
    let rung = if flush {
        stated
            .iter()
            .find(|rung| *rung != rungs::IMMEDIATELY)
            .or_else(|| stated.first())
            .cloned()
    } else {
        stated
            .iter()
            .find(|rung| *rung == rungs::IMMEDIATELY)
            .cloned()
    };
    let Some(rung) = rung else {
        case.subject
            .release_tool_call(&held_call.arguments)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(Ending::Inapplicable(if flush {
            "the session states no rung for tools".into()
        } else {
            "the session supports no immediately rung for tools; flush_prefix_applies_pending \
             checks the next-run rule with a flush"
                .into()
        }));
    };
    let Some(plan) = plan_for(case, &session.name, RefreshVariant::ChangedTools) else {
        case.subject
            .release_tool_call(&held_call.arguments)
            .await
            .map_err(|e| e.to_string())?;
        return Ok(Ending::Inapplicable(
            "the adapter supplies no refresh plan with another tool set".into(),
        ));
    };
    // Whatever the checks made while the call is held find, release it, so
    // a failing runner is not left with a run that never ends.
    let checked = async {
        let first = request_at(
            &requests(case, &session.name).await?,
            0,
            "the held run's first step",
        )?;
        changed_plan(&first, &plan)?;
        let (answer, body) = accepted(&session.route, &plan.plan, &rung, 1).await?;
        if !answer.run_in_progress {
            return Err(format!(
                "a refresh while a run is running answered {body}, with run_in_progress: false"
            ));
        }
        if !flush && !answer.applies_from_next_run {
            return Err(format!(
                "a refresh with the immediately rung answered {body}, with applies_from_next_run: \
                 false"
            ));
        }
        if flush {
            let send_id = case.mint.next("flush");
            let request = serde_json::to_value(FlushPrefixRequest::new(&send_id)).unwrap();
            let first_answer = expect_ok(
                "session.flush_prefix",
                call(&session.route, ops::SESSION_FLUSH_PREFIX, request.clone()).await?,
            )?;
            let flushed: FlushPrefixReply = decode("session.flush_prefix", first_answer.clone())?;
            if flushed.pending_generation != Some(1) {
                return Err(format!(
                    "session.flush_prefix with generation 1 pending answered {first_answer}, not \
                     pending_generation 1"
                ));
            }
            let repeated = expect_ok(
                "session.flush_prefix repeat",
                call(&session.route, ops::SESSION_FLUSH_PREFIX, request).await?,
            )?;
            if repeated != first_answer {
                return Err(format!(
                    "a repeat of session.flush_prefix with send_id {send_id} answered {repeated}, \
                     not {first_answer}"
                ));
            }
        }
        Ok::<Tools, String>(first)
    }
    .await;
    case.subject
        .release_tool_call(&held_call.arguments)
        .await
        .map_err(|e| format!("releasing the held call: {e}"))?;
    let first = checked?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let seen = requests(case, &session.name).await?;
    let later = request_at(&seen, 1, "the held run's step after the release")?;
    if later != first {
        return Err(format!(
            "the running run's step after the {} offered {later:?}, not the tools {first:?} \
             it started with: the change applied inside a running run",
            if flush { "flush" } else { "refresh" }
        ));
    }
    run_once(case, &mut session).await?;
    let next = request_at(&requests(case, &session.name).await?, 2, "the next run")?;
    if !uses_new_plan(&next, &first, &plan) {
        return Err(format!(
            "the run after the {} offered {next:?}, not the new plan's tools {:?}",
            if flush { "flush" } else { "refresh" },
            plan.tools
        ));
    }
    applied(&session.route, 1).await?;
    Ok(Ending::Passed)
}

/// A session with one finished run, its first request's tools and the
/// cheapest rung it states for tools.
async fn idle<S>(
    case: &Case<'_, S>,
    label: &str,
) -> Result<(Session<S::Route>, Vec<String>), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            label,
            Script {
                turns: vec![text("first run"), text("must not start")],
            },
        )
        .await?;
    run_once(case, &mut session).await?;
    let stated = tool_rungs(&baseline(&session.route).await?)?;
    if stated.is_empty() {
        return Err("the session states no rung for tools".into());
    }
    Ok((session, stated))
}

/// An identical repeat gets the same answer and changes nothing.
async fn repeat<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, stated) = idle(case, "change-repeat").await?;
    let Some(plan) = plan_for(case, &session.name, RefreshVariant::ChangedTools) else {
        return Ok(Ending::Inapplicable(
            "the adapter supplies no refresh plan with another tool set".into(),
        ));
    };
    let (_, first) = accepted(&session.route, &plan.plan, &stated[0], 1).await?;
    let before = baseline(&session.route).await?;
    let again = expect_ok(
        "a repeated session.refresh",
        refresh(&session.route, &plan.plan, &stated[0], 1).await?,
    )?;
    if again != first {
        return Err(format!(
            "an identical repeat of session.refresh answered {again}, not {first}"
        ));
    }
    let after = baseline(&session.route).await?;
    if after != before {
        return Err(format!(
            "an identical repeat changed session.baseline from {before:?} to {after:?}"
        ));
    }
    Ok(Ending::Passed)
}

/// A refresh at the accepted generation with another policy or plan, or
/// below it, is refused `generation_stale {accepted}` and writes nothing.
async fn stale<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, stated) = idle(case, "change-stale").await?;
    let Some(plan) = plan_for(case, &session.name, RefreshVariant::ChangedTools) else {
        return Ok(Ending::Inapplicable(
            "the adapter supplies no refresh plan with another tool set".into(),
        ));
    };
    accepted(&session.route, &plan.plan, &stated[0], 2).await?;
    let before = baseline(&session.route).await?;
    // The same generation with the session's first plan, then the same
    // generation with another rung where the session states one, then a
    // lower generation with the accepted plan and rung.
    let first_plan: Map<String, Value> = case
        .subject
        .send_fields(&session.name, true)
        .get("plan")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut attempts = vec![
        (
            "the accepted generation with another plan",
            first_plan,
            stated[0].clone(),
            2,
        ),
        (
            "a lower generation",
            plan.plan.clone(),
            stated[0].clone(),
            1,
        ),
    ];
    if let Some(other) = stated.get(1) {
        attempts.push((
            "the accepted generation with another policy",
            plan.plan.clone(),
            other.clone(),
            2,
        ));
    }
    for (what, plan, policy, generation) in attempts {
        let error = expect_error(
            what,
            refresh(&session.route, &plan, &policy, generation).await?,
        )?;
        expect_code(what, &error, errors::GENERATION_STALE)?;
        let detail: GenerationStaleDetail = decode(
            "the generation_stale detail",
            error.detail.clone().unwrap_or(Value::Null),
        )?;
        if detail != GenerationStaleDetail::new(2) {
            return Err(format!(
                "{what} was refused generation_stale with {detail:?}, not accepted: 2"
            ));
        }
        let after = baseline(&session.route).await?;
        if after != before {
            return Err(format!(
                "{what}, refused generation_stale, changed session.baseline from {before:?} \
                 to {after:?}"
            ));
        }
    }
    Ok(Ending::Passed)
}

/// On a host that fixes its tool set at start, a tool outside it, and a
/// kept name with a changed definition, are refused by name, writing
/// nothing.
async fn outside_host_set<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (session, stated) = idle(case, "change-host-set").await?;
    let outside = plan_for(case, &session.name, RefreshVariant::OutsideHostSet);
    let changed = plan_for(case, &session.name, RefreshVariant::ChangedDefinition);
    let (outside, changed) = match (outside, changed) {
        (None, None) => {
            return Ok(Ending::Inapplicable(
                "the adapter's host does not fix its tool set at start".into(),
            ))
        }
        (Some(outside), Some(changed)) => (outside, changed),
        _ => {
            return Err(
                "the adapter supplies only one of the OutsideHostSet and ChangedDefinition \
                 refresh plans; a host that fixes its tool set fixes each tool's definition \
                 with it, so both apply"
                    .into(),
            )
        }
    };
    let before = baseline(&session.route).await?;
    for (what, plan) in [
        ("a tool outside the host's set", outside),
        ("a kept name with a changed definition", changed),
    ] {
        // Generation 1 both times: a refused refresh accepts nothing, so the
        // second is not stale.
        let error = expect_error(
            what,
            refresh(&session.route, &plan.plan, &stated[0], 1).await?,
        )?;
        expect_code(what, &error, errors::TOOL_OUTSIDE_HOST_SET)?;
        let detail: ToolOutsideHostSetDetail = decode(
            "the tool_outside_host_set detail",
            error.detail.clone().unwrap_or(Value::Null),
        )?;
        let named: Tools = detail.names.iter().cloned().collect();
        let wanted: Tools = plan.tools.iter().cloned().collect();
        if named != wanted {
            return Err(format!(
                "{what} was refused tool_outside_host_set naming {:?}, not {wanted:?}",
                detail.names
            ));
        }
        let after = baseline(&session.route).await?;
        if after != before {
            return Err(format!(
                "{what}, refused tool_outside_host_set, changed session.baseline from \
                 {before:?} to {after:?}"
            ));
        }
    }
    Ok(Ending::Passed)
}
