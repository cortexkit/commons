//! Session spend and next-send model observations, read from the real route.

use cortexkit_role_llm_runner::{
    capabilities as groups, ops,
    send::SendReply,
    usage::{RunUsage, SessionUsage},
};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, head, run_result, send_params, wait_run_end},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedTurn, ScriptedUsage},
};

fn totals(usage: &RunUsage) -> [Option<u64>; 5] {
    [
        usage.input_tokens,
        usage.cached_input_tokens,
        usage.cache_write_tokens,
        usage.output_tokens,
        usage.reasoning_tokens,
    ]
}

fn spend(head: &cortexkit_role_llm_runner::read::HeadMeta) -> Result<&SessionUsage, String> {
    let usage = head
        .usage
        .as_ref()
        .ok_or("session.head omitted declared session_usage")?;
    if !usage.consistent() {
        return Err(format!("session usage is inconsistent: {usage:?}"));
    }
    Ok(usage)
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let model_case = name == "head_model_reflects_recorded_switch";
    if !model_case && !case.subject.session_usage_fixtures() {
        return Ok(Ending::Inapplicable(
            "adapter has no session-usage fixtures".into(),
        ));
    }
    // Distinct fields expose totals accidentally copied from the wrong field.
    // No step reports reasoning tokens, even though every other field is known.
    let mut session = case
        .open(
            "session-usage",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("spend-first").0])
                        .with_usage(ScriptedUsage::new(11, 101, 7, 13)),
                    ScriptedTurn::new(vec![case.mint.text("spend-second").0])
                        .with_usage(ScriptedUsage::new(17, 211, 3, 19)),
                ],
            },
        )
        .await?;
    if model_case {
        let prompt = case.mint.next("model-prompt");
        let send_id = case.mint.next("model-send");
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
        wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
        let before = head(&session.route).await?.model;
        if before.is_none() {
            return Ok(Ending::Inapplicable(
                "runner does not report head.model".into(),
            ));
        }
        let Some(expected) = case
            .subject
            .switch_head_model(&session.name)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(Ending::Inapplicable(
                "adapter cannot record a model switch".into(),
            ));
        };
        if before.as_ref() == Some(&expected) {
            return Err("model switch fixture returned the unchanged original selection".into());
        }
        let got = head(&session.route).await?.model;
        if got.as_ref() != Some(&expected) {
            return Err(format!(
                "head.model did not reflect recorded switch: got {got:?}, expected {expected:?}"
            ));
        }
        return Ok(Ending::Passed);
    }
    crate::tags::require_declared(&session.route, groups::SESSION_USAGE, ops::SESSION_HEAD).await?;
    let mut runs = Vec::new();
    for first in [true, false] {
        let prompt = case.mint.next("spend-prompt");
        let send_id = case.mint.next("spend-send");
        let reply: SendReply = decode(
            "session.send",
            expect_ok(
                "session.send",
                call(
                    &session.route,
                    ops::SESSION_SEND,
                    send_params(case.subject, &session.name, first, &prompt, &send_id, None),
                )
                .await?,
            )?,
        )?;
        if first {
            session.first = Some((send_id, prompt));
        }
        let before = runs.last().map(|(id, _): &(String, RunUsage)| id.as_str());
        let end = wait_run_end(case.subject, &session.route, case.declared, before, &reply).await?;
        let usage = run_result(&session.route, &end.run_id)
            .await?
            .usage
            .ok_or("run.result omitted run_usage")?;
        if usage.steps == 0 || !usage.consistent() {
            return Err(format!(
                "fixture run had no completed, consistent usage: {usage:?}"
            ));
        }
        runs.push((end.run_id, usage));
    }
    if runs[0].0 == runs[1].0 {
        return Err("fixture did not create two runs".into());
    }
    let snapshot = head(&session.route).await?;
    let usage = spend(&snapshot)?;
    match name {
        "session_usage_sums_runs_since_first" => {
            if usage.since != runs[0].0 {
                return Err(format!(
                    "since {:?} is not first counted run {:?}",
                    usage.since, runs[0].0
                ));
            }
            if usage.totals.steps != runs.iter().map(|(_, u)| u.steps).sum::<u64>()
                || usage.totals.unreported_steps
                    != runs.iter().map(|(_, u)| u.unreported_steps).sum::<u64>()
            {
                return Err("session step counts do not sum over both runs".into());
            }
            for (index, actual) in totals(&usage.totals).into_iter().enumerate() {
                // An absent value is not zero: compare sums only when every
                // step in both runs reported the scripted field.
                if index < 4 {
                    let a =
                        totals(&runs[0].1)[index].ok_or("scripted field absent from first run")?;
                    let b =
                        totals(&runs[1].1)[index].ok_or("scripted field absent from second run")?;
                    if actual != Some(a + b) {
                        return Err(format!(
                            "session token field {index} {actual:?} does not sum to {}",
                            a + b
                        ));
                    }
                }
            }
        }
        "session_usage_unknown_fields_absent" => {
            if usage.totals.reasoning_tokens.is_some()
                || usage
                    .totals
                    .by_model
                    .iter()
                    .any(|m| m.reasoning_tokens.is_some())
            {
                return Err(
                    "reasoning_tokens must be absent: no scripted step reported it, never zero"
                        .into(),
                );
            }
        }
        "session_usage_rewind_never_decreases" => {
            if !case
                .subject
                .rewind_session_usage(&session.name)
                .await
                .map_err(|e| e.to_string())?
            {
                return Ok(Ending::Inapplicable(
                    "host cannot rewind visible history".into(),
                ));
            }
            let after = head(&session.route).await?;
            if after.history_version <= snapshot.history_version {
                return Err(
                    "rewind fixture did not rewrite visible history (history_version did not rise)"
                        .into(),
                );
            }
            if spend(&after)? != usage {
                return Err(format!(
                    "rewind changed durable spend: before {usage:?}, after {:?}",
                    after.usage
                ));
            }
        }
        _ => return Err(format!("unknown session usage case {name}")),
    }
    Ok(Ending::Passed)
}
