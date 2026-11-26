//! Plan-note checks over management routes and independently observed requests.

use cortexkit_role_llm_runner::{
    baseline::Baseline,
    capabilities, errors, ops,
    plan_notes::{tool_removal, PlanNotesRefreshRequest, NOTES_UNTIL_REBUILD},
    session_change::RefreshReply,
};
use serde_json::{json, Map};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_error, expect_field, expect_ok, read_all},
    send_helpers::send_and_wait,
    session_change::baseline,
    subject::{
        LlmRunnerSubject, ModelRequestObservation, PlanNotesFixture, Script, ScriptedPart,
        ScriptedTurn,
    },
    Capability, RunnerRoute,
};

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let declared = describe.declares(capabilities::PLAN_NOTES);
    if name == "plan_notes_requires_session_change" {
        return Ok(if declared {
            Ending::Passed
        } else {
            Ending::Inapplicable("plan_notes is not declared".into())
        });
    }
    let undeclared = name == "plan_notes_undeclared_policy_refused";
    if declared == undeclared {
        return Ok(Ending::Inapplicable(if declared {
            "plan_notes is declared".into()
        } else {
            "plan_notes is not declared".into()
        }));
    }
    let Some(fixture) = case.subject.plan_notes_fixture() else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no plan-note fixture".into(),
        ));
    };
    if !case.declared.contains(&Capability::Queue)
        || !case.declared.contains(&Capability::TranscriptReads)
        || !case.declared.contains(&Capability::RunOps)
    {
        return Ok(Ending::Inapplicable(
            "plan-note fixture requires queue, transcript_reads and run_ops".into(),
        ));
    }
    if fixture.system_note.is_empty() || fixture.system_note == fixture.new_system_text {
        return Err(
            "plan-note fixture must distinguish a short note from replacement system text".into(),
        );
    }
    let restoration = match name {
        "plan_notes_restore_after_delivery" => Some(Restoration::Delivered),
        "plan_notes_restore_before_delivery" => Some(Restoration::Undelivered),
        "plan_notes_restore_requires_system_note" => Some(Restoration::MissingNote),
        _ => None,
    };
    if let Some(restoration) = restoration {
        return restore(case, name, &fixture, restoration).await;
    }
    let removed = name == "plan_notes_removed_tool_refused";
    let mut tool = case.mint.call("removed-tool");
    tool.tool = fixture.removed_tool.clone();
    let turns = vec![
        ScriptedTurn::new(vec![ScriptedPart::Text("first answer".into())]),
        ScriptedTurn::new(if removed {
            vec![ScriptedPart::ToolCall(tool.clone())]
        } else {
            vec![ScriptedPart::Text("second answer".into())]
        }),
        ScriptedTurn::new(vec![ScriptedPart::Text("final answer".into())]),
        ScriptedTurn::new(vec![ScriptedPart::Text("after flush".into())]),
    ];
    let mut session = case.open(name, Script { turns }).await?;
    send_and_wait(case, &mut session, None, "start", &Map::new()).await?;
    let before = baseline(&session.route).await?;
    let first = case
        .subject
        .model_requests(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    if !first
        .first()
        .is_some_and(|request| request.tools.contains(&fixture.removed_tool))
    {
        return Err("plan-note fixture did not initially offer the removed tool".into());
    }
    let request = PlanNotesRefreshRequest::new(
        fixture.plan,
        NOTES_UNTIL_REBUILD,
        before.accepted_generation + 1,
    )
    .with_system_note(&fixture.system_note);
    let reply = call(&session.route, ops::SESSION_REFRESH, json!(request)).await?;
    if undeclared {
        let error = expect_error("undeclared note policy", reply)?;
        if error.code != errors::INVALID_PARAMS
            || error
                .detail
                .as_ref()
                .and_then(|d| d.get("field"))
                .and_then(|f| f.as_str())
                != Some("system_note")
        {
            expect_field(
                "undeclared note policy",
                &error,
                errors::INVALID_PARAMS,
                "policy",
            )?;
        }
        // Probe the policy alone too, so rejecting the unknown member cannot
        // hide a runner that would silently accept an undeclared policy.
        let mut policy_only = json!(request);
        policy_only.as_object_mut().unwrap().remove("system_note");
        let error = expect_error(
            "undeclared note policy",
            call(&session.route, ops::SESSION_REFRESH, policy_only).await?,
        )?;
        expect_field(
            "undeclared note policy",
            &error,
            errors::INVALID_PARAMS,
            "policy",
        )?;
        return Ok(Ending::Passed);
    }
    let reply: RefreshReply = decode("note refresh", expect_ok("note refresh", reply)?)?;
    if reply.accepted_generation != request.generation
        || reply.pending.generation != request.generation
        || reply.pending.policy != NOTES_UNTIL_REBUILD
        || reply.applies_from_next_run
    {
        return Err("stable-prefix note refresh did not leave the full plan pending".into());
    }
    send_and_wait(case, &mut session, None, "deliver notes", &Map::new()).await?;
    let requests = case
        .subject
        .model_requests(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let next = requests
        .get(1)
        .ok_or("note-delivery run made no model request")?;
    let visible = next
        .rendered
        .iter()
        .chain(next.system_text.iter().flatten())
        .any(|text| text.contains(&fixture.system_note));
    if !visible && name != "plan_notes_system_note_not_full_text" {
        return Err("note-delivery request did not contain the system note".into());
    }
    match name {
        "cleared_ride_plan_notes_never_clear" => {
            let cleared = next
                .cleared_system_text
                .as_ref()
                .ok_or("plan-note request did not observe clearing markers")?;
            if !cleared.is_empty() {
                return Err("plan note used turn_system_cleared".into());
            }
            if !next
                .system_text
                .as_ref()
                .is_some_and(|texts| texts.iter().any(|text| text.contains(&fixture.system_note)))
            {
                return Err("plan note did not retain persistent system authority".into());
            }
        }
        "plan_notes_pending_until_reconcile" => {
            let pending = baseline(&session.route).await?;
            if pending.applied_generation != before.applied_generation
                || pending.effective != before.effective
                || pending.pending.as_ref().is_none_or(|p| {
                    p.generation != request.generation || p.policy != NOTES_UNTIL_REBUILD
                })
                || next.tools != first[0].tools
            {
                return Err(
                    "notes changed the applied surface/generation before a reconcile point".into(),
                );
            }
            expect_ok(
                "flush notes",
                call(
                    &session.route,
                    ops::SESSION_FLUSH_PREFIX,
                    json!({"send_id":case.mint.next("flush")}),
                )
                .await?,
            )?;
            send_and_wait(case, &mut session, None, "reconcile", &Map::new()).await?;
            let applied = baseline(&session.route).await?;
            if applied.applied_generation != request.generation || applied.pending.is_some() {
                return Err("flush did not fully apply pending notes".into());
            }
            let requests = case
                .subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            if requests.last().is_some_and(|r| {
                r.rendered
                    .iter()
                    .chain(r.system_text.iter().flatten())
                    .any(|t| t.contains(&fixture.system_note))
            }) {
                return Err("reconciled request still renders plan notes".into());
            }
        }
        "plan_notes_not_stored_entries" => {
            let (_, messages) = read_all(&session.route).await?;
            if messages
                .iter()
                .any(|m| m.message.to_string().contains(&fixture.system_note))
            {
                return Err("read exposed a plan note as a stored entry".into());
            }
            if describe.declares(capabilities::HOST_HISTORY) {
                let history = expect_ok(
                    "note history",
                    call(
                        &session.route,
                        ops::SESSION_HISTORY,
                        json!({"from_ordinal":1,"limit":100}),
                    )
                    .await?,
                )?;
                if history["messages"]
                    .to_string()
                    .contains(&fixture.system_note)
                {
                    return Err("history exposed a plan note as a stored entry".into());
                }
            }
        }
        "plan_notes_removed_tool_refused" => {
            if case.subject.tool_invocations(&tool.arguments).await != 0 {
                return Err("removed tool reached the provider".into());
            }
            let (_, messages) = read_all(&session.route).await?;
            if !messages.iter().any(|m| {
                m.message
                    .to_string()
                    .contains(&tool_removal(&fixture.removed_tool).replace('"', "\\\""))
            }) {
                return Err("removed tool refusal did not name the removal".into());
            }
        }
        "plan_notes_system_note_not_full_text" => {
            if !visible
                || next
                    .rendered
                    .iter()
                    .chain(next.system_text.iter().flatten())
                    .any(|text| text.contains(&fixture.new_system_text))
            {
                return Err(
                    "model received full replacement system text instead of the short system note"
                        .into(),
                );
            }
        }
        _ => return Err(format!("unknown plan-note case {name}")),
    }
    Ok(Ending::Passed)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Restoration {
    Delivered,
    Undelivered,
    MissingNote,
}

async fn restore<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
    fixture: &PlanNotesFixture,
    scenario: Restoration,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let Some((plan, correction)) = &fixture.restoration else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no restoring plan or correction".into(),
        ));
    };
    if plan == &fixture.plan
        || correction.is_empty()
        || correction.contains(&fixture.system_note)
        || fixture.system_note.contains(correction)
        || fixture.new_system_text.contains(correction)
    {
        return Err(
            "restoration fixture must supply a different plan and a distinct correction".into(),
        );
    }
    let script = Script {
        turns: (1..=4)
            .map(|n| ScriptedTurn::new(vec![ScriptedPart::Text(format!("answer {n}"))]))
            .collect(),
    };
    let mut session = case.open(name, script).await?;
    let extra = Map::new();
    send_and_wait(case, &mut session, None, "start", &extra).await?;
    let original = baseline(&session.route).await?;
    let requests = case
        .subject
        .model_requests(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let initial = requests
        .first()
        .ok_or("initial run made no model request")?;
    if !initial.tools.contains(&fixture.removed_tool)
        || !note_parts(initial, &fixture.system_note).is_empty()
        || !note_parts(initial, correction).is_empty()
        || !note_parts(initial, &fixture.new_system_text).is_empty()
    {
        return Err("initial request did not offer the original surface without notes".into());
    }

    let removal = PlanNotesRefreshRequest::new(
        fixture.plan.clone(),
        NOTES_UNTIL_REBUILD,
        original.accepted_generation + 1,
    )
    .with_system_note(&fixture.system_note);
    let reply = call(&session.route, ops::SESSION_REFRESH, json!(removal)).await?;
    let reply: RefreshReply = decode("removing refresh", expect_ok("removing refresh", reply)?)?;
    if reply.accepted_generation != removal.generation || reply.applies_from_next_run {
        return Err("removing refresh did not leave the full plan pending".into());
    }

    let delivered = if scenario == Restoration::Undelivered {
        None
    } else {
        send_and_wait(case, &mut session, None, "deliver removal", &extra).await?;
        let requests = case
            .subject
            .model_requests(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        let next = requests.get(1).ok_or("removal run made no model request")?;
        if next.tools != initial.tools
            || note_parts(next, &fixture.system_note).is_empty()
            || !note_parts(next, correction).is_empty()
            || !note_parts(next, &fixture.new_system_text).is_empty()
        {
            return Err(
                "removal run did not deliver just the first note on the original surface".into(),
            );
        }
        Some(next.clone())
    };
    let pending = baseline(&session.route).await?;
    if !still_pending(&pending, &original, removal.generation) {
        return Err("removing generation was not pending before its restoration".into());
    }

    let restoring =
        PlanNotesRefreshRequest::new(plan.clone(), NOTES_UNTIL_REBUILD, removal.generation + 1);
    if scenario != Restoration::Delivered {
        let label = "restoring refresh without system_note";
        let error = expect_error(
            label,
            call(&session.route, ops::SESSION_REFRESH, json!(restoring)).await?,
        )?;
        expect_field(label, &error, errors::INVALID_PARAMS, "system_note")?;
        if baseline(&session.route).await? != pending {
            return Err("missing-note refusal changed the baseline".into());
        }
        if scenario == Restoration::MissingNote {
            return Ok(Ending::Passed);
        }
    }
    let restoring = restoring.with_system_note(correction);
    let reply = call(&session.route, ops::SESSION_REFRESH, json!(restoring)).await?;
    let reply: RefreshReply = decode("restoring refresh", expect_ok("restoring refresh", reply)?)?;
    if reply.accepted_generation != restoring.generation {
        return Err("restoring refresh did not accept the requested generation".into());
    }
    send_and_wait(case, &mut session, None, "restore", &extra).await?;
    let restored = baseline(&session.route).await?;
    let requests = case
        .subject
        .model_requests(&session.name)
        .await
        .map_err(|e| e.to_string())?;
    let next = requests
        .get(if delivered.is_some() { 2 } else { 1 })
        .ok_or("restoring run made no model request")?;
    if next.tools != initial.tools || !note_parts(next, &fixture.new_system_text).is_empty() {
        return Err("restoring run did not render the original surface".into());
    }
    if let Some(delivered) = delivered {
        if note_parts(next, &fixture.system_note) != note_parts(&delivered, &fixture.system_note) {
            return Err(
                "restoration dropped or edited the delivered note before reconciliation".into(),
            );
        }
        if note_parts(next, correction).is_empty() {
            return Err(
                "restoring run did not deliver the owner's correction as a new note".into(),
            );
        }
        if reply.applies_from_next_run || !still_pending(&restored, &original, restoring.generation)
        {
            return Err("restoring generation fully applied before reconciliation".into());
        }
        expect_ok(
            "flush notes",
            call(
                &session.route,
                ops::SESSION_FLUSH_PREFIX,
                json!({"send_id":case.mint.next("flush")}),
            )
            .await?,
        )?;
        send_and_wait(case, &mut session, None, "reconcile", &extra).await?;
        let applied = baseline(&session.route).await?;
        let requests = case
            .subject
            .model_requests(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        let next = requests
            .get(3)
            .ok_or("reconcile run made no model request")?;
        check_applied_restoration(
            &applied,
            next,
            restoring.generation,
            initial,
            fixture,
            correction,
        )?;
    } else {
        if !reply.applies_from_next_run {
            return Err("restoration was deferred even though no note was delivered".into());
        }
        check_applied_restoration(
            &restored,
            next,
            restoring.generation,
            initial,
            fixture,
            correction,
        )?;
    }
    Ok(Ending::Passed)
}

fn note_parts<'a>(request: &'a ModelRequestObservation, note: &str) -> Vec<&'a str> {
    request
        .rendered
        .iter()
        .chain(request.system_text.iter().flatten())
        .filter(|text| text.contains(note))
        .map(String::as_str)
        .collect()
}

fn still_pending(current: &Baseline, original: &Baseline, generation: u64) -> bool {
    current.accepted_generation == generation
        && current.applied_generation == original.applied_generation
        && current.effective == original.effective
        && current.pending.as_ref().is_some_and(|pending| {
            pending.generation == generation && pending.policy == NOTES_UNTIL_REBUILD
        })
}

fn check_applied_restoration(
    applied: &Baseline,
    next: &ModelRequestObservation,
    generation: u64,
    initial: &ModelRequestObservation,
    fixture: &PlanNotesFixture,
    correction: &str,
) -> Result<(), String> {
    if applied.applied_generation != generation || applied.pending.is_some() {
        return Err("restoring generation did not fully apply".into());
    }
    if next.tools != initial.tools
        || !note_parts(next, &fixture.new_system_text).is_empty()
        || !note_parts(next, &fixture.system_note).is_empty()
        || !note_parts(next, correction).is_empty()
    {
        return Err("applied restoration did not render the original surface without notes".into());
    }
    Ok(())
}
