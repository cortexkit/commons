//! `run_usage` cases, run only when the subject declares `run_usage`: a
//! run's token usage on `run.result` and `run.status` (contract §6.4).

use cortexkit_role_llm_runner::{
    capabilities as groups, ops,
    run::{RunStatus, RunUsage},
    send::SendReply,
};
use serde_json::json;

use crate::{
    cases::Case,
    drive::{call, decode, expect_ok, run_result, send_params, wait_run_end},
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn, ScriptedUsage},
};

type Check = Result<(), String>;

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    match name {
        "run_usage_counts_reported_steps" => counts_reported_steps(case).await,
        other => Err(format!("unknown run_usage case {other}")),
    }
}

/// The five totals of `usage`, named, in wire order.
fn totals(usage: &RunUsage) -> [(&'static str, Option<u64>); 5] {
    [
        ("input_tokens", usage.input_tokens),
        ("cached_input_tokens", usage.cached_input_tokens),
        ("cache_write_tokens", usage.cache_write_tokens),
        ("output_tokens", usage.output_tokens),
        ("reasoning_tokens", usage.reasoning_tokens),
    ]
}

/// What the run's `usage` must say after the two scripted steps: both
/// reported, each total the sum of what the script reported, and
/// `reasoning_tokens`, which the script never reports, absent.
fn check_usage(what: &str, usage: &RunUsage, steps: &[ScriptedUsage]) -> Check {
    let sum = |field: fn(&ScriptedUsage) -> u64| Some(steps.iter().map(field).sum::<u64>());
    let expected = [
        ("input_tokens", sum(|s| s.input_tokens)),
        ("cached_input_tokens", sum(|s| s.cache_read_tokens)),
        ("cache_write_tokens", sum(|s| s.cache_write_tokens)),
        ("output_tokens", sum(|s| s.output_tokens)),
        ("reasoning_tokens", None),
    ];
    if usage.steps != steps.len() as u64 || usage.unreported_steps != 0 {
        return Err(format!(
            "{what} usage counts {} steps with {} unreported, but the run completed {} steps \
             and the scripted model reported usage for every one: {usage:?}",
            usage.steps,
            usage.unreported_steps,
            steps.len()
        ));
    }
    for ((name, got), (_, want)) in totals(usage).into_iter().zip(expected) {
        match (got, want) {
            (Some(0), None) => {
                return Err(format!(
                    "{what} usage reports {name} as 0, but the scripted model reported no \
                     {name} for any step: a total no step reported must be absent, never 0"
                ))
            }
            (got, want) if got != want => {
                return Err(format!(
                    "{what} usage reports {name} {got:?}, but the scripted model reported \
                     {want:?} over the run's steps"
                ))
            }
            _ => {}
        }
    }
    if usage.by_model.is_empty() || !usage.consistent() {
        return Err(format!(
            "{what} usage's by_model does not sum to its steps and totals (one entry per \
             provider, model and variant, each with a step): {usage:?}"
        ));
    }
    Ok(())
}

async fn counts_reported_steps<S>(case: &Case<'_, S>) -> Check
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    // Two steps, a tool call and then the final text, with distinct counts
    // per field and per step, so a total taken from one step or from the
    // wrong field differs from the sum. Neither step reports reasoning.
    let reported = [
        ScriptedUsage::new(11, 101, 7, 13),
        ScriptedUsage::new(17, 211, 3, 19),
    ];
    let tool_call = case.mint.call("usage-call");
    let (last, _) = case.mint.text("usage-final");
    let mut session = case
        .open(
            "run-usage",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool_call)])
                        .with_usage(reported[0].clone()),
                    ScriptedTurn::new(vec![last]).with_usage(reported[1].clone()),
                ],
            },
        )
        .await?;
    crate::tags::require_declared(&session.route, groups::RUN_USAGE, ops::RUN_RESULT).await?;
    let prompt = case.mint.next("usage-prompt");
    let send_id = case.mint.next("usage-send");
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(
                &session.route,
                ops::SESSION_SEND,
                send_params(case.subject, &session.name, true, &prompt, &send_id, None),
            )
            .await?,
        )?,
    )?;
    session.first = Some((send_id, prompt));
    let end = wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let result = run_result(&session.route, &end.run_id).await?;
    let Some(usage) = &result.usage else {
        return Err(format!(
            "run.result of an ended run carries no usage, though the runner declares \
             run_usage: {result:?}"
        ));
    };
    check_usage("run.result", usage, &reported)?;
    if case.declared.contains(&Capability::RunStatus) {
        // The ended run has completed steps, so run.status must carry usage
        // too. Before its first step completes, usage may be present or
        // absent, so the case does not read run.status then.
        let status: RunStatus = decode(
            "run.status",
            expect_ok(
                "run.status",
                call(
                    &session.route,
                    ops::RUN_STATUS,
                    json!({"run_id": end.run_id}),
                )
                .await?,
            )?,
        )?;
        if status.usage.as_ref() != Some(usage) {
            return Err(format!(
                "run.status of the ended run carries usage {:?}, but run.result carries \
                 {usage:?}",
                status.usage
            ));
        }
    }
    Ok(())
}
