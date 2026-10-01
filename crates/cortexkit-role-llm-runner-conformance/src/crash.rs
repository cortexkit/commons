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
    errors::tool_result_reasons,
    ops, points,
    read::{HeadMeta, ReadMessage},
    run::{RunResult, RunState},
    send::SendReply,
};

use crate::{
    cases::entries_for,
    drive::{
        call, check_unique, decode, expect_ok, head, holding, read_all, result_marker, run_result,
        send_params, wait_run_end, Mint, MAX_POLLS,
    },
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn},
};

/// The run-ops case that has no scenario of its own: it reads `run.result`
/// of the run the `DispatchIntent` kill cut, because a run can cut each
/// point only once.
const INTERRUPTED_CASE: &str = "run_result_interrupted_not_cancelled";

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
}

/// The kill point a case's scenario cuts at, if it is a crash case.
pub fn point_of(case: &str) -> Option<&'static str> {
    if case == INTERRUPTED_CASE {
        return Some(points::DISPATCH_INTENT);
    }
    let name = case.strip_prefix("crash_at_")?;
    points::ALL.iter().copied().find(|point| *point == name)
}

fn turn(parts: Vec<ScriptedPart>) -> ScriptedTurn {
    ScriptedTurn { parts }
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
    install(
        name_b.clone(),
        Script {
            turns: vec![
                turn(vec![b_step, ScriptedPart::ToolCall(b_call.clone())]),
                turn(vec![b_final]),
            ],
        },
    )
    .await?;

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
    let prompt = mint.next("prompt");
    let params_b = send_params(subject, &name_b, true, &prompt, &mint.next("send"), None);
    let trigger: Trigger<'_> = {
        let route_b = &route_b;
        let params = params_b.clone();
        Box::pin(async move {
            if route_b.request(ops::SESSION_SEND, params).await.is_err() {
                return;
            }
            for _ in 0..MAX_POLLS {
                match head(route_b).await {
                    Err(_) => return,
                    Ok(head) => {
                        if head
                            .last_run_state
                            .is_some_and(|last| RunState::parse(&last.state).is_terminal())
                        {
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
    wait_run_end(subject, &route_b, declared, None, &retry).await?;
    let (_, after_a) = read_all(&route_a).await?;
    let (_, after_b) = read_all(&route_b).await?;
    let head_b = head(&route_b).await?;
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
    })
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
                match entries_for(b, &observed.call.tool_call_id).as_slice() {
                    [entry] if !entry.indeterminate => {}
                    other => {
                        return Err(format!(
                        "the never-sent call must have indeterminate: false; entries are {other:?}"
                    ))
                    }
                }
                expect_holding(b, &observed.step_text, 1, "the durable step's text")?;
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
                    match entries_for(follow_up, &observed.call.tool_call_id).as_slice() {
                        [entry] if !entry.indeterminate => {}
                        _ => {
                            return Err(
                                "the call must remain indeterminate: false after the follow-up"
                                    .into(),
                            )
                        }
                    }
                    result_before_turn(
                        follow_up,
                        &observed.call.tool_call_id,
                        &observed.final_text,
                    )?;
                } else {
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

/// Result bodies are runner-specific. Join a non-call message to the call
/// through its model id or attributed call key, without inspecting result content.
fn result_before_turn(
    messages: &[ReadMessage],
    call_id: &str,
    later_text: &str,
) -> Result<(), String> {
    fn contains_id(value: &serde_json::Value, id: &str) -> bool {
        match value {
            serde_json::Value::String(text) => text == id,
            serde_json::Value::Array(values) => values.iter().any(|v| contains_id(v, id)),
            serde_json::Value::Object(values) => values.values().any(|v| contains_id(v, id)),
            _ => false,
        }
    }
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
