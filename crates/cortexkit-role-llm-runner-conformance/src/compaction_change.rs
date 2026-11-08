//! Management-route observations of compaction plan refreshes. Plan fixtures come from
//! the adapter, so these probes never invent a provider or its configuration.
//! Baselines expose generations and manifest digests, not raw plans or views;
//! provider Setup and preservation of an applied view are not observable here.

use cortexkit_role_llm_runner::{errors, ops, rungs, session_change::RefreshReply};
use serde_json::{Map, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_error, expect_field, expect_ok, head, wait_run_end},
    route::{Reply, RunnerRoute},
    session_change::{applied, baseline, refresh, run_once},
    subject::{LlmRunnerSubject, RefreshVariant, Script, ScriptedPart, ScriptedTurn},
};

fn item(plan: &Map<String, Value>) -> Option<&Value> {
    plan.get("compaction_item").filter(|item| !item.is_null())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::Text("before refresh".into())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("after refresh".into())]),
                ],
            },
        )
        .await?;
    let fields = case.subject.send_fields(&session.name, true);
    let full = case
        .subject
        .compaction_change_plan(&session.name)
        .or_else(|| fields.get("plan").and_then(Value::as_object).cloned());
    let Some(full) = full.filter(|plan| item(plan).is_some()) else {
        return Ok(Ending::Inapplicable(
            "neither compaction_change_plan nor its send_fields fallback supplies a compaction item"
                .into(),
        ));
    };
    let original_item = item(&full).unwrap();
    let adding = name == "compaction_add_takes_effect_next_run";
    let removing = name == "compaction_remove_is_refused";
    let mut opening = full.clone();
    let mut replacement = full.clone();
    if adding {
        opening.remove("compaction_item");
    } else if removing {
        replacement.remove("compaction_item");
    } else {
        let switched = case
            .subject
            .compaction_switch_plan(&session.name)
            .or_else(|| {
                case.subject
                    .refresh_plan(&session.name, RefreshVariant::ChangedTools)
                    .map(|plan| plan.plan)
            });
        let Some(switched) =
            switched.filter(|plan| item(plan).is_some_and(|other| other != original_item))
        else {
            return Ok(Ending::Inapplicable(
                "neither compaction_switch_plan nor its refresh_plan(ChangedTools) fallback \
                 supplies a different compaction item"
                    .into(),
            ));
        };
        replacement = switched;
    }
    let send_id = case.mint.next("compaction-start");
    let mut params = case.params(&session, "before refresh", &send_id, None);
    params["plan"] = Value::Object(opening);
    let sent = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first = Some((send_id, "before refresh".into()));
    wait_run_end(case.subject, &session.route, case.declared, None, &sent).await?;
    let before = baseline(&session.route).await?;
    let before_head = head(&session.route).await?;
    if !adding && !removing {
        let Some(rungs) = &before.rungs else {
            return Ok(Ending::Inapplicable(
                "the session states no refresh rungs".into(),
            ));
        };
        if !rungs.tools.iter().any(|rung| rung == rungs::IMMEDIATELY) {
            return Ok(Ending::Inapplicable(
                "the session supports no immediately rung for the replacement plan's tools".into(),
            ));
        }
    }
    let response = refresh(&session.route, &replacement, rungs::IMMEDIATELY, 1).await?;
    if removing || matches!(response, Reply::Error(_)) {
        let error = expect_error("compaction-item refresh", response)?;
        expect_field(
            "compaction-item refresh",
            &error,
            errors::INVALID_PARAMS,
            "plan.compaction_item",
        )?;
        if adding {
            return Err("adding a compaction item must be accepted, not refused".into());
        }
        if baseline(&session.route).await? != before || head(&session.route).await? != before_head {
            return Err(
                "compaction-item refusal changed the applied baseline or session head".into(),
            );
        }
        return Ok(Ending::Passed);
    }
    let first = expect_ok("compaction-item refresh", response)?;
    let answer: RefreshReply = decode("compaction-item refresh", first.clone())?;
    if answer.accepted_generation != 1
        || answer.pending.generation != 1
        || answer.pending.policy != rungs::IMMEDIATELY
        || answer.run_in_progress
        || !answer.applies_from_next_run
    {
        return Err(format!(
            "idle compaction-item refresh must accept generation 1 for the next run: {first}"
        ));
    }
    let pending = baseline(&session.route).await?;
    if pending.accepted_generation != 1
        || pending.applied_generation != before.applied_generation
        || pending.effective != before.effective
        || pending.pending.as_ref() != Some(&answer.pending)
    {
        return Err("compaction-item refresh did not stay pending until the next run".into());
    }
    run_once(case, &mut session).await?;
    applied(&session.route, 1).await?;
    if baseline(&session.route)
        .await?
        .effective
        .is_none_or(|effective| effective.manifest_digest.is_empty())
    {
        return Err("the applied compaction change has no manifest digest".into());
    }
    Ok(Ending::Passed)
}
