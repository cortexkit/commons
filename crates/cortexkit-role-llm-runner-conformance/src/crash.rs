//! The crash cases: kill the runner at one of the role's points, restart it
//! on the same root, and check what the role's crash guarantees
//! (`CONTRACT.md` §14 in `cortexkit-role-llm-runner`) promise for that
//! point: ids and keys are never reused or changed, a run gets one terminal
//! state and a cut run is never `cancelled`, a tool call is dispatched at
//! most once, and a retried send writes nothing twice.
//!
//! Each point is cut once per run, on its own root, in one scenario. Two
//! sessions run on the root: session A runs to its end and is read before
//! the kill, so its pages can be compared after the restart; session B's
//! first send is the trigger the kill cuts. After the restart the suite
//! retries B's send with the same `send_id`, waits for B's run to end, and
//! records what it read. Every case that needs the point checks that one
//! record.

use std::{collections::BTreeSet, path::Path};

use cortexkit_role_harness::{CrashDriver, KillPoint, Trigger};
use cortexkit_role_llm_runner::{
    capabilities as groups,
    errors::{self, tool_result_reasons},
    ops, points,
    read::{HeadMeta, ReadMessage, ReadRequest},
    run::{RunResult, RunState},
    send::SendReply,
    tags::{self, ResolveReply, ResolveRequest},
};
use serde_json::Value;

use crate::{
    cases::{entries_for, Ending},
    drive::{
        call, check_unique, decode, expect_code, expect_ok, head, holding, read_all, read_refusal,
        result_marker, run_result, send_params, wait_run_end, Mint, MAX_POLLS,
    },
    route::{Reply, RunnerRoute},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn},
    tags::{entry_holding, require_declared},
};

/// The run-ops case that has no scenario of its own: it reads `run.result`
/// of the run the `DispatchIntent` kill cut, because a run can cut each
/// point only once.
const INTERRUPTED_CASE: &str = "run_result_interrupted_not_cancelled";

/// The tags case that has no scenario of its own: it resolves tags in the
/// session the `StepRecorded` kill cut, after a seal that left its lineage.
pub const LEFT_LINEAGE_TAG_CASE: &str = "tags_resolve_answers_the_current_lineage";

/// The most tag numbers [`LEFT_LINEAGE_TAG_CASE`] resolves, so a long
/// session is not sent thousands of requests.
const MAX_RESOLVED_TAGS: usize = 64;

/// What one crash scenario observed.
pub struct CrashObservation {
    /// Session A's transcript before the kill, and after the restart.
    before_a: Vec<ReadMessage>,
    after_a: Vec<ReadMessage>,
    /// Session B's transcript and head after the restart, once its run ended.
    after_b: Vec<ReadMessage>,
    head_b: HeadMeta,
    /// The reply to B's first send, retried after the restart.
    retry: SendReply,
    /// How often the tool provider ran B's call by the kill, and in all.
    invocations_at_kill: usize,
    invocations_after: usize,
    /// B's run, when the subject declares `run_ops`.
    run_result: Option<RunResult>,
    /// B's prompt, the text of its first step, its call, and the text of
    /// its final step.
    prompt: String,
    step_text: String,
    call: ScriptedToolCall,
    final_text: String,
    /// After a run ends interrupted without dispatch, send another owner prompt.
    /// Store the resulting transcript or the error from sending, waiting or reading;
    /// absent when recovery did not take this sealing branch.
    sealed_follow_up: Option<Result<Vec<ReadMessage>, String>>,
    /// At `StepRecorded`, session B's lineage after an earlier finished run,
    /// read before the kill, and its lineage after the restart. A recovery
    /// that drops or rewrites the cut step must move B to a new lineage.
    lineage_before_b: Option<String>,
    lineage_after_b: Option<String>,
    /// At `StepRecorded`, session B's transcript read before the kill, after
    /// its earlier run ended. A recovery that keeps B's lineage must read
    /// these messages back unchanged.
    before_b: Vec<ReadMessage>,
    /// At `StepRecorded`, when B's lineage changed across the restart: `Ok`
    /// when a read naming the old lineage was refused `lineage_changed`, or
    /// what was answered instead. Absent when the lineage did not change.
    old_lineage_read: Option<Result<(), String>>,
    /// At `StepRecorded`, with `tags` declared, after a seal that moved B to
    /// a new lineage and an owner follow-up on it: `Ok` with the tags no
    /// part of the new lineage carries, each refused `unknown_tag`, or what
    /// was answered instead. Absent when the scenario took no such branch.
    left_lineage_tags: Option<Result<Vec<String>, String>>,
}

/// The kill point a case's scenario cuts at, if it is a crash case.
pub fn point_of(case: &str) -> Option<&'static str> {
    if case == INTERRUPTED_CASE {
        return Some(points::DISPATCH_INTENT);
    }
    if case == LEFT_LINEAGE_TAG_CASE {
        return Some(points::STEP_RECORDED);
    }
    let name = case.strip_prefix("crash_at_")?;
    points::ALL.iter().copied().find(|point| *point == name)
}

fn turn(parts: Vec<ScriptedPart>) -> ScriptedTurn {
    ScriptedTurn::new(parts)
}

/// Run the scenario for `point` on a fresh root.
pub async fn observe<S>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
    point: &'static str,
) -> Result<CrashObservation, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let root = work_dir.join(format!("crash-{point}"));
    std::fs::create_dir_all(&root).map_err(|e| format!("creating {}: {e}", root.display()))?;
    let owner = subject.owner_stamp();
    let install = |name: String, script: Script| async move {
        subject
            .install_script(&name, script)
            .await
            .map_err(|e| format!("installing the script of {name}: {e}"))
    };

    let name_a = mint.next(&format!("crash-{point}-a"));
    let (a_step, _) = mint.text("answer");
    let (a_final, _) = mint.text("answer");
    install(
        name_a.clone(),
        Script {
            turns: vec![
                turn(vec![a_step, ScriptedPart::ToolCall(mint.call("call_a"))]),
                turn(vec![a_final]),
            ],
        },
    )
    .await?;
    let name_b = mint.next(&format!("crash-{point}-b"));
    let (b_step, step_text) = mint.text("step");
    let (b_final, final_text) = mint.text("final");
    let b_call = mint.call("call_b");
    // At StepRecorded, B first runs one finished turn, so its lineage can be
    // read before the kill and compared after the recovery.
    let earlier_b = point == points::STEP_RECORDED;
    let mut b_turns = Vec::new();
    if earlier_b {
        b_turns.push(turn(vec![mint.text("earlier").0]));
    }
    b_turns.push(turn(vec![b_step, ScriptedPart::ToolCall(b_call.clone())]));
    b_turns.push(turn(vec![b_final]));
    install(name_b.clone(), Script { turns: b_turns }).await?;

    let handle = driver
        .spawn(&root)
        .await
        .map_err(|e| format!("spawning: {e}"))?;
    let route_a = subject
        .session_route(handle.inner(), &name_a, &owner)
        .await
        .map_err(|e| format!("opening the route of {name_a}: {e}"))?;
    let prompt_a = mint.next("prompt");
    let params_a = send_params(subject, &name_a, true, &prompt_a, &mint.next("send"), None);
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session A's send",
            call(&route_a, ops::SESSION_SEND, params_a).await?,
        )?,
    )?;
    wait_run_end(subject, &route_a, declared, None, &reply).await?;
    let (_, before_a) = read_all(&route_a).await?;

    let route_b = subject
        .session_route(handle.inner(), &name_b, &owner)
        .await
        .map_err(|e| format!("opening the route of {name_b}: {e}"))?;
    let mut lineage_before_b = None;
    let mut earlier_run = None;
    let mut before_b = Vec::new();
    if earlier_b {
        let mut params = send_params(
            subject,
            &name_b,
            true,
            &mint.next("earlier"),
            &mint.next("send"),
            None,
        );
        // On a tags runner B renders tags, so the left-lineage case has
        // numbers to resolve: numbering applies only to such sessions.
        if declared.contains(&Capability::Tags) {
            if let Ok(fields) = crate::tags::rendering_fields(subject, &name_b) {
                let mut rendering = Value::Object(fields);
                rendering["prompt"] = params["prompt"].clone();
                rendering["send_id"] = params["send_id"].clone();
                params = rendering;
            }
        }
        let reply: SendReply = decode(
            "session.send",
            expect_ok(
                "session B's earlier send",
                call(&route_b, ops::SESSION_SEND, params).await?,
            )?,
        )?;
        wait_run_end(subject, &route_b, declared, None, &reply).await?;
        let head_before = head(&route_b).await?;
        lineage_before_b = head_before.lineage_id;
        earlier_run = head_before.last_run_state.map(|last| last.run_id);
        before_b = read_all(&route_b).await?.1;
    }
    let prompt = mint.next("prompt");
    let params_b = send_params(
        subject,
        &name_b,
        !earlier_b,
        &prompt,
        &mint.next("send"),
        None,
    );
    let trigger: Trigger<'_> = {
        let route_b = &route_b;
        let params = params_b.clone();
        let earlier_run = earlier_run.clone();
        Box::pin(async move {
            if route_b.request(ops::SESSION_SEND, params).await.is_err() {
                return;
            }
            for _ in 0..MAX_POLLS {
                match head(route_b).await {
                    Err(_) => return,
                    Ok(head) => {
                        if head.last_run_state.is_some_and(|last| {
                            Some(&last.run_id) != earlier_run.as_ref()
                                && RunState::parse(&last.state).is_terminal()
                        }) {
                            return;
                        }
                    }
                }
                subject.pause().await;
            }
        })
    };
    drop(route_a);
    driver
        .kill_at(handle, &KillPoint::new(point), trigger)
        .await
        .map_err(|e| format!("killing at {point}: {e}"))?;
    drop(route_b);
    let invocations_at_kill = subject.tool_invocations(&b_call.arguments).await;

    let handle = driver
        .restart(&root)
        .await
        .map_err(|e| format!("restarting after {point}: {e}"))?;
    let route_a = subject
        .session_route(handle.inner(), &name_a, &owner)
        .await
        .map_err(|e| format!("reopening {name_a}: {e}"))?;
    let route_b = subject
        .session_route(handle.inner(), &name_b, &owner)
        .await
        .map_err(|e| format!("reopening {name_b}: {e}"))?;
    let retry: SendReply = decode(
        "the retried send",
        expect_ok(
            "B's first send, retried after the restart",
            call(&route_b, ops::SESSION_SEND, params_b).await?,
        )?,
    )?;
    wait_run_end(subject, &route_b, declared, earlier_run.as_deref(), &retry).await?;
    let (_, after_a) = read_all(&route_a).await?;
    let (_, after_b) = read_all(&route_b).await?;
    let head_b = head(&route_b).await?;
    let head_b_lineage = head_b.lineage_id.clone();
    let old_lineage_read = match &lineage_before_b {
        Some(old) if head_b_lineage.as_ref() != Some(old) => {
            let what = "a read naming B's lineage from before the kill";
            let request = ReadRequest::tail().with_lineage_id(old);
            Some(
                read_refusal(&route_b, what, &request)
                    .await
                    .and_then(|error| expect_code(what, &error, errors::LINEAGE_CHANGED)),
            )
        }
        _ => None,
    };
    let invocations_after = subject.tool_invocations(&b_call.arguments).await;
    let run_result = match (
        &head_b.last_run_state,
        declared.contains(&Capability::RunOps),
    ) {
        (Some(last), true) => Some(run_result(&route_b, &last.run_id).await?),
        _ => None,
    };
    let sealed_follow_up = if point == points::STEP_RECORDED
        && invocations_after == 0
        && head_b
            .last_run_state
            .as_ref()
            .is_some_and(|last| RunState::parse(&last.state) == RunState::Interrupted)
    {
        // One assistant step is already durable, so the follow-up consumes
        // the script's second turn, just as resumption would have done.
        Some(
            async {
                let reply: SendReply = decode(
                    "the owner follow-up",
                    expect_ok(
                        "the owner follow-up",
                        call(
                            &route_b,
                            ops::SESSION_SEND,
                            send_params(
                                subject,
                                &name_b,
                                false,
                                &mint.next("follow-up"),
                                &mint.next("send"),
                                None,
                            ),
                        )
                        .await?,
                    )?,
                )?;
                wait_run_end(subject, &route_b, declared, None, &reply).await?;
                let (_, messages) = read_all(&route_b).await?;
                if subject.tool_invocations(&b_call.arguments).await != 0 {
                    return Err("the never-sent call was dispatched on the follow-up".into());
                }
                Ok(messages)
            }
            .await,
        )
    } else {
        None
    };
    let left_lineage_tags = match (&sealed_follow_up, &old_lineage_read) {
        (Some(Ok(follow_up)), Some(_)) if declared.contains(&Capability::Tags) => {
            Some(resolve_left_lineage(&route_b, follow_up, &prompt).await)
        }
        (Some(Err(error)), Some(_)) if declared.contains(&Capability::Tags) => Some(Err(format!(
            "the owner follow-up on the new lineage failed, so its tags could not be read: {error}"
        ))),
        _ => None,
    };
    Ok(CrashObservation {
        before_a,
        after_a,
        after_b,
        head_b,
        retry,
        invocations_at_kill,
        invocations_after,
        run_result,
        prompt,
        step_text,
        call: b_call,
        final_text,
        sealed_follow_up,
        lineage_before_b,
        lineage_after_b: head_b_lineage,
        before_b,
        old_lineage_read,
        left_lineage_tags,
    })
}

/// In the lineage a seal moved session B to, resolve the numbers its parts
/// carry (at most [`MAX_RESOLVED_TAGS`] of them): each answers the part of the
/// new lineage that shows it, never a part of the lineage the session left,
/// though numbers may repeat across lineages. A number above the new
/// lineage's highest is refused `unknown_tag`, and the cut run's prompt
/// resolves where it now sits. Answers the numbers resolved.
async fn resolve_left_lineage<R: RunnerRoute>(
    route: &R,
    messages: &[ReadMessage],
    prompt: &str,
) -> Result<Vec<String>, String> {
    require_declared(route, groups::TAGS, ops::SESSION_RESOLVE).await?;
    let carried: Vec<(String, &ReadMessage, String)> = messages
        .iter()
        .flat_map(|m| {
            m.tags
                .iter()
                .filter_map(move |row| Some((row.tag.clone()?, m, row.part.clone())))
        })
        .take(MAX_RESOLVED_TAGS)
        .collect();
    let mut resolved = Vec::new();
    for (tag, entry, part) in &carried {
        let what = format!("session.resolve of {tag}, which the new lineage shows");
        let reply: ResolveReply = decode(
            &what,
            expect_ok(
                &what,
                call(
                    route,
                    ops::SESSION_RESOLVE,
                    serde_json::to_value(ResolveRequest::new(tag)).expect("serializes"),
                )
                .await?,
            )?,
        )?;
        if reply.ordinal != entry.ordinal || reply.mid != entry.mid || &reply.part != part {
            return Err(format!(
                "{what} answered ordinal {} mid {} part {}, but the new lineage reads it on \
                 ordinal {} mid {} part {part}: resolve answers the current lineage, never the \
                 one the session left",
                reply.ordinal, reply.mid, reply.part, entry.ordinal, entry.mid
            ));
        }
        resolved.push(tag.clone());
    }
    let highest = carried
        .iter()
        .filter_map(|(tag, _, _)| tags::parse_tag(tag))
        .max()
        .unwrap_or(0);
    let above = tags::format_tag(highest + 1).expect("positive");
    let what = format!("session.resolve of {above}, above every number the new lineage shows");
    match call(
        route,
        ops::SESSION_RESOLVE,
        serde_json::to_value(ResolveRequest::new(&above)).expect("serializes"),
    )
    .await?
    {
        Reply::Response(body) => {
            return Err(format!(
                "{what} answered {body}: a number no part of the current lineage carries is \
                 refused unknown_tag, never answered with a left lineage's content"
            ))
        }
        Reply::Error(error) => expect_code(&what, &error, errors::UNKNOWN_TAG)?,
    }
    let entry = entry_holding(messages, prompt)?;
    if !entry.tags.iter().any(|row| row.tag.is_some()) {
        return Err(format!(
            "the cut run's prompt at ordinal {} carries no number in the new lineage",
            entry.ordinal
        ));
    }
    Ok(resolved)
}

/// [`LEFT_LINEAGE_TAG_CASE`], against the `StepRecorded` scenario.
pub fn check_left_lineage(observed: &CrashObservation) -> Result<Ending, String> {
    match &observed.left_lineage_tags {
        Some(Ok(resolved)) if resolved.is_empty() => Ok(Ending::Inapplicable(
            "the seal moved the session to a new lineage that shows no number, so there is \
             nothing to resolve"
                .into(),
        )),
        Some(Ok(_)) => Ok(Ending::Passed),
        Some(Err(reason)) => Err(reason.clone()),
        None => Ok(Ending::Inapplicable(
            "the StepRecorded recovery resumed the run or kept the never-sent call on the same \
             lineage (shape (b)), so the session left no lineage whose tags could be asked for"
                .into(),
        )),
    }
}

/// Check `case` against the scenario observed at `point`.
pub fn check(
    case: &str,
    point: &str,
    observed: &CrashObservation,
    declared: &BTreeSet<Capability>,
) -> Result<(), String> {
    if case == INTERRUPTED_CASE {
        let result = observed
            .run_result
            .as_ref()
            .ok_or("the scenario read no run.result for the cut run")?;
        return match result.run_state() {
            RunState::Interrupted => Ok(()),
            _ => Err(format!(
                "the run cut at {point} answers run.result {}, not interrupted",
                result.state
            )),
        };
    }
    common(point, observed).map_err(|reason| {
        if point == points::STEP_RECORDED {
            let branch = if observed.invocations_after == 0 {
                "sealed"
            } else {
                "resume"
            };
            format!("{branch} branch: {reason}")
        } else {
            reason
        }
    })?;
    let b = &observed.after_b;
    let last = observed
        .head_b
        .last_run_state
        .as_ref()
        .expect("common checked the last run");
    let invoked = observed.invocations_after;
    let result_text = result_marker(&observed.call);
    match point {
        points::STEP_RECORDED => {
            // Zero observed tool invocations selects sealing; any invocation
            // selects resumption, whose at-most-once limit is checked below.
            let branch = if invoked == 0 { "sealed" } else { "resume" };
            let check_branch = || -> Result<(), String> {
                if observed.invocations_at_kill != 0 {
                    return Err(format!(
                        "the call ran {} times before its dispatch intent was durable",
                        observed.invocations_at_kill
                    ));
                }
                if invoked > 1 {
                    return Err(format!(
                        "at-most-once dispatch broke: the call ran {invoked} times"
                    ));
                }
                if invoked == 0 {
                    if RunState::parse(&last.state) != RunState::Interrupted {
                        return Err(format!(
                            "zero dispatches require interrupted, not {}",
                            last.state
                        ));
                    }
                    expect_holding(b, &observed.final_text, 0, "a later model turn's text")?;
                    let follow_up = observed
                        .sealed_follow_up
                        .as_ref()
                        .ok_or("no owner follow-up was observed")?
                        .as_ref()
                        .map_err(Clone::clone)?;
                    check_sealed_history(observed, b, follow_up)?;
                } else {
                    expect_kept_step(b, observed)?;
                    expect_holding(b, &observed.final_text, 1, "the continued run's final text")?;
                    expect_holding(b, &result_text, 1, "the dispatched call's result")?;
                    result_before_turn(b, &observed.call.tool_call_id, &observed.final_text)?;
                }
                Ok(())
            };
            check_branch().map_err(|reason| format!("{branch} branch: {reason}"))?;
        }
        points::DISPATCH_INTENT => {
            if observed.invocations_at_kill > 1 || invoked != observed.invocations_at_kill {
                return Err(format!(
                    "the call with a durable dispatch intent ran {} times by the kill and {invoked} times in all; it is never re-dispatched",
                    observed.invocations_at_kill
                ));
            }
            if RunState::parse(&last.state) != RunState::Interrupted {
                return Err(format!(
                    "the run cut at its dispatch intent ended {}, not interrupted",
                    last.state
                ));
            }
            if holding(b, tool_result_reasons::OUTCOME_UNKNOWN) == 0 {
                return Err("no message closes the cut call with outcome_unknown".into());
            }
            expect_holding(b, &observed.final_text, 0, "a later model turn's text")?;
            if declared.contains(&Capability::DispatchAttribution) {
                match entries_for(b, &observed.call.tool_call_id).as_slice() {
                    [entry] if !entry.indeterminate && entry.call_key.is_some() => {}
                    other => {
                        return Err(format!(
                            "after its outcome_unknown close the cut call's entries are {other:?}, not one closed entry with its call_key"
                        ))
                    }
                }
                let keys = |messages: &[ReadMessage]| -> Vec<(u64, String, Option<String>)> {
                    messages
                        .iter()
                        .flat_map(|m| {
                            m.tool_calls.iter().map(move |e| {
                                (m.ordinal, e.tool_call_id.clone(), e.call_key.clone())
                            })
                        })
                        .collect()
                };
                let before = keys(&observed.before_a);
                let after: Vec<_> = keys(&observed.after_a)
                    .into_iter()
                    .filter(|(ordinal, ..)| observed.before_a.iter().any(|m| m.ordinal == *ordinal))
                    .collect();
                if before.is_empty() || before != after {
                    return Err(format!(
                        "call_keys read before the kill {before:?} differ after it: {after:?}"
                    ));
                }
            }
        }
        points::TOOL_RESULT_RECORDED => {
            if invoked != 1 {
                return Err(format!(
                    "the call whose result was durable ran {invoked} times; resume replays it without calling the tool again"
                ));
            }
            expect_holding(b, &result_text, 1, "the recorded tool result")?;
            if declared.contains(&Capability::DispatchAttribution)
                && entries_for(b, &observed.call.tool_call_id)
                    .iter()
                    .any(|entry| entry.indeterminate)
            {
                return Err("the call whose result was durable is read as indeterminate".into());
            }
        }
        points::TERMINAL => {
            if RunState::parse(&last.state) != RunState::Completed {
                return Err(format!(
                    "the run whose completed terminal was durable reads {} after the restart",
                    last.state
                ));
            }
            if invoked != 1 {
                return Err(format!("the finished run's call ran {invoked} times"));
            }
            expect_holding(b, &observed.final_text, 1, "the final turn's text")?;
        }
        _ => {}
    }
    Ok(())
}

/// The guarantees every point shares: session A's messages read before the
/// kill read the same after it, ordinals and mids are each used once, B's
/// retried send is answered and nothing of B is written twice, and B's run
/// has one terminal state that is not `cancelled`.
fn common(point: &str, observed: &CrashObservation) -> Result<(), String> {
    for message in &observed.before_a {
        match observed
            .after_a
            .iter()
            .find(|after| after.ordinal == message.ordinal)
        {
            Some(after) if after == message => {}
            other => {
                return Err(format!(
                    "ordinal {} read {message:?} before the kill at {point} and {other:?} after it",
                    message.ordinal
                ))
            }
        }
    }
    check_unique("session A after the restart", &observed.after_a)?;
    check_unique("session B after the restart", &observed.after_b)?;
    let b = &observed.after_b;
    expect_holding(b, &observed.prompt, 1, "the retried prompt")?;
    for (marker, what) in [
        (&observed.step_text, "the first step's text"),
        (&result_marker(&observed.call), "the tool result"),
        (&observed.final_text, "the final step's text"),
    ] {
        let copies = holding(b, marker);
        if copies > 1 {
            return Err(format!("{what} is written in {copies} messages"));
        }
    }
    let last = observed
        .head_b
        .last_run_state
        .as_ref()
        .ok_or("session B has no last run after the restart")?;
    let state = RunState::parse(&last.state);
    if state == RunState::Cancelled {
        return Err(format!(
            "the run cut by the kill at {point} ended cancelled; a cut run is never cancelled"
        ));
    }
    if !state.is_terminal() {
        return Err(format!(
            "session B's last run is {}, not terminal",
            last.state
        ));
    }
    if let Some(run_id) = &observed.retry.run_id {
        if run_id != &last.run_id {
            return Err(format!(
                "the retried send names run {run_id}, but the session's last run is {}",
                last.run_id
            ));
        }
    }
    Ok(())
}

fn expect_holding(
    messages: &[ReadMessage],
    marker: &str,
    copies: usize,
    what: &str,
) -> Result<(), String> {
    let found = holding(messages, marker);
    if found == copies {
        Ok(())
    } else {
        Err(format!(
            "{what} is written in {found} messages, not {copies}"
        ))
    }
}

fn contains_id(value: &serde_json::Value, id: &str) -> bool {
    match value {
        serde_json::Value::String(text) => text == id,
        serde_json::Value::Array(values) => values.iter().any(|v| contains_id(v, id)),
        serde_json::Value::Object(values) => values.values().any(|v| contains_id(v, id)),
        _ => false,
    }
}

fn expect_kept_step(messages: &[ReadMessage], observed: &CrashObservation) -> Result<(), String> {
    match entries_for(messages, &observed.call.tool_call_id).as_slice() {
        [entry] if !entry.indeterminate => {}
        other => {
            return Err(format!(
                "the never-sent call must have indeterminate: false; entries are {other:?}"
            ))
        }
    }
    expect_holding(messages, &observed.step_text, 1, "the durable step's text")
}

/// A run sealed `interrupted` around a durable step whose call was never
/// sent has two valid shapes (`CONTRACT.md` §14, property 4), and the
/// follow-up must preserve the chosen one so no dangling call enters later
/// model history:
///
/// - (a) drop the call, which rewrites a written message and so moves the
///   session to a new lineage, whose old reads are refused `lineage_changed`;
/// - (b) keep the call and append its never-executed result in the commit
///   that ends the run, which only appends and so keeps the lineage.
///
/// The cut-run transcript decides the shape: no attribution entry for the
/// call is shape (a), an entry is shape (b). A failure names the shape it
/// was checked against.
fn check_sealed_history(
    observed: &CrashObservation,
    cut: &[ReadMessage],
    follow_up: &[ReadMessage],
) -> Result<(), String> {
    let id = &observed.call.tool_call_id;
    let keys: Vec<_> = cut
        .iter()
        .chain(follow_up)
        .flat_map(|m| &m.tool_calls)
        .filter(|entry| entry.tool_call_id == *id)
        .filter_map(|entry| entry.call_key.as_deref())
        .collect();
    let carries_call = |m: &ReadMessage| {
        contains_id(&m.message, id) || keys.iter().any(|key| contains_id(&m.message, key))
    };
    if entries_for(cut, id).is_empty() {
        check_dropped(observed, cut, follow_up, &carries_call).map_err(|reason| {
            format!("closest to shape (a), the call dropped on a new lineage: {reason}")
        })
    } else {
        check_kept(observed, cut, follow_up, &carries_call).map_err(|reason| {
            format!(
                "closest to shape (b), the call kept and closed in the sealing commit: {reason}"
            )
        })
    }
}

/// Shape (a): the never-sent call is gone from the cut-run and follow-up
/// transcripts, and the session is on a new lineage whose predecessor's
/// reads are refused.
fn check_dropped(
    observed: &CrashObservation,
    cut: &[ReadMessage],
    follow_up: &[ReadMessage],
    carries_call: &dyn Fn(&ReadMessage) -> bool,
) -> Result<(), String> {
    let id = &observed.call.tool_call_id;
    let text_copies = holding(cut, &observed.step_text);
    if text_copies > 1 {
        return Err("the dropped call's assistant text is held more than once".into());
    }
    for messages in [cut, follow_up] {
        if !entries_for(messages, id).is_empty() || messages.iter().any(carries_call) {
            return Err("the dropped call remains in attribution or message history".into());
        }
        expect_holding(
            messages,
            &result_marker(&observed.call),
            0,
            "the dropped call's result",
        )?;
        expect_holding(
            messages,
            &observed.step_text,
            text_copies,
            "the dropped call's assistant text",
        )?;
    }
    expect_holding(
        follow_up,
        &observed.final_text,
        1,
        "the later assistant turn",
    )?;
    // Dropping the call (and maybe its text) rewrote a written message,
    // so the recovery must have started a new lineage (§4.2, §14).
    if observed.lineage_before_b.is_none() || observed.lineage_after_b == observed.lineage_before_b
    {
        return Err(format!(
            "the recovery dropped the never-sent call from history but kept lineage {:?}: \
             dropping or rewriting a written message starts a new lineage",
            observed.lineage_after_b
        ));
    }
    match &observed.old_lineage_read {
        Some(Ok(())) => Ok(()),
        Some(Err(reason)) => Err(format!(
            "the lineage moved from {:?} to {:?}, but {reason}",
            observed.lineage_before_b, observed.lineage_after_b
        )),
        None => Err("the lineage moved, but no read of the old lineage was observed".into()),
    }
}

/// Shape (b): the lineage is unchanged and every message read before the
/// kill reads back the same; the call is kept, not indeterminate, and the
/// message right after it carries its result, already in the transcript
/// read once the run ended. The follow-up only appends: the sealed
/// transcript reads back unchanged and no message it writes carries the
/// call, so the result was not written by a later commit.
fn check_kept(
    observed: &CrashObservation,
    cut: &[ReadMessage],
    follow_up: &[ReadMessage],
    carries_call: &dyn Fn(&ReadMessage) -> bool,
) -> Result<(), String> {
    let id = &observed.call.tool_call_id;
    match (&observed.lineage_before_b, &observed.lineage_after_b) {
        (Some(before), Some(after)) if before == after => {}
        (before, after) => {
            return Err(format!(
                "the kept call needs the lineage unchanged, but it read {before:?} before the kill \
                 and {after:?} after it"
            ))
        }
    }
    for message in &observed.before_b {
        let after = cut.iter().find(|after| after.ordinal == message.ordinal);
        if after != Some(message) {
            return Err(format!(
                "ordinal {} read {message:?} before the kill and {after:?} after the seal under \
                 the same lineage: a seal that keeps the lineage only appends, so a message \
                 already read never changes",
                message.ordinal
            ));
        }
    }
    expect_kept_step(cut, observed)?;
    let calls: Vec<_> = cut
        .iter()
        .enumerate()
        .filter(|(_, m)| !m.tool_calls.is_empty() && carries_call(m))
        .collect();
    let [(at, call_message)] = calls.as_slice() else {
        return Err(format!(
            "the kept call is carried by {} messages, not one",
            calls.len()
        ));
    };
    let results = cut
        .iter()
        .filter(|m| m.tool_calls.is_empty() && carries_call(m))
        .count();
    let next = cut.get(at + 1);
    if results != 1 || !next.is_some_and(|m| m.tool_calls.is_empty() && carries_call(m)) {
        return Err(format!(
            "dangling call {id}: once the run ended interrupted, the message right after the call \
             at ordinal {} must carry its never-executed result, written in the commit that ended \
             the run; found ordinal {:?} next and {results} messages carrying a result",
            call_message.ordinal,
            next.map(|m| m.ordinal)
        ));
    }
    for message in cut {
        let after = follow_up
            .iter()
            .find(|after| after.ordinal == message.ordinal);
        if !after.is_some_and(|after| after.mid == message.mid && after.message == message.message)
        {
            return Err(format!(
                "ordinal {} read {message:?} once the run ended and {after:?} after the owner's \
                 follow-up",
                message.ordinal
            ));
        }
    }
    let written_later: Vec<u64> = follow_up
        .iter()
        .filter(|m| !cut.iter().any(|sealed| sealed.ordinal == m.ordinal) && carries_call(m))
        .map(|m| m.ordinal)
        .collect();
    if !written_later.is_empty() {
        return Err(format!(
            "dangling call {id} after the run ended: it is closed by ordinals {written_later:?}, \
             written later; its result belongs in the commit that ended the run, not a later one"
        ));
    }
    expect_kept_step(follow_up, observed)?;
    expect_holding(
        follow_up,
        &observed.final_text,
        1,
        "the later assistant turn",
    )
}

/// Result bodies are runner-specific. Join a non-call message to the call
/// through its model id or attributed call key, without inspecting result content.
fn result_before_turn(
    messages: &[ReadMessage],
    call_id: &str,
    later_text: &str,
) -> Result<(), String> {
    expect_holding(messages, later_text, 1, "the later assistant turn")?;
    let later = messages
        .iter()
        .find(|m| holding(std::slice::from_ref(m), later_text) == 1)
        .expect("checked later turn");
    let entries = entries_for(messages, call_id);
    let key = entries.first().and_then(|entry| entry.call_key.as_deref());
    let results: Vec<_> = messages
        .iter()
        .filter(|m| {
            m.tool_calls.is_empty()
                && (contains_id(&m.message, call_id)
                    || key.is_some_and(|key| contains_id(&m.message, key)))
        })
        .collect();
    match results.as_slice() {
        [result] if result.ordinal < later.ordinal => Ok(()),
        _ => Err(format!("dangling call {call_id}: expected one result before the later assistant turn at ordinal {}, found result ordinals {:?}",
            later.ordinal, results.iter().map(|m| m.ordinal).collect::<Vec<_>>())),
    }
}
