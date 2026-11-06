//! Plan-note checks over management routes and independently observed requests.

use cortexkit_role_llm_runner::{
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
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
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
