//! Live-route command checks observing provider calls, model inputs and host menus.

use std::{collections::BTreeSet, path::Path};

use cortexkit_role_harness::{CrashDriver, KillPoint, Trigger};
use cortexkit_role_llm_runner::{
    baseline::BaselineReply,
    capabilities as groups, errors, ops, points,
    send::SendReply,
    user_commands::{UserCommandAnswer, UserCommandStatus},
};
use serde_json::{json, Value};

use crate::{
    cases::Case,
    drive::{call, decode, expect_code, expect_error, expect_ok, read_all, wait_run_end, Mint},
    route::RunnerRoute,
    subject::{
        Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn, UserCommandFixture,
    },
};

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::Text(
                        "ordinary model answer".into(),
                    )]);
                    2
                ],
            },
        )
        .await?;
    crate::tags::require_declared(
        &session.route,
        groups::USER_COMMANDS,
        ops::SESSION_USER_COMMAND,
    )
    .await?;
    let fixture = UserCommandFixture {
        collision: name == "user_commands_collision",
        interactive: name != "user_commands_no_person",
        host_replies_outside_history: name != "user_commands_unsafe_host_menu",
    };
    let (session_ref, fields) = case
        .subject
        .prepare_user_commands(&session.name, &fixture)
        .await
        .map_err(|e| e.to_string())?;
    let send_id = case.mint.next("command-admit");
    let mut params = Value::Object(fields);
    params["prompt"] = json!("ordinary prompt");
    params["send_id"] = json!(send_id);
    let admission = call(&session.route, ops::SESSION_SEND, params).await?;
    if fixture.collision {
        let error = expect_error("colliding admission", admission)?;
        expect_code("command collision", &error, errors::USER_COMMAND_COLLISION)?;
        let detail = error.detail.ok_or("collision has no detail")?;
        if detail["name"] != "ctx-status"
            || detail["providers"]
                .as_array()
                .is_none_or(|p| p.len() < 2 || p.iter().any(|p| !p.is_string()) || p[0] == p[1])
        {
            return Err(format!(
                "collision must name the command and both providers: {detail}"
            ));
        }
        return Ok(());
    }
    let send: SendReply = decode("admission", expect_ok("admission", admission)?)?;
    session.first = Some((send_id, "ordinary prompt".into()));
    wait_run_end(case.subject, &session.route, case.declared, None, &send).await?;
    let baseline: BaselineReply = decode(
        "baseline",
        expect_ok(
            "baseline",
            call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
        )?,
    )?;
    let frozen = baseline.baseline.ok_or("no frozen baseline")?.user_commands;
    if !fixture.interactive {
        if !frozen.is_empty() {
            return Err("no-person baseline listed commands".into());
        }
        let reply = call(&session.route, ops::SESSION_USER_COMMAND, json!({"session_ref": session_ref, "command": "ctx-status", "input": "", "invocation_id": case.mint.next("no-person")})).await?;
        return expect_code(
            "no-person invocation",
            &expect_error("no-person invocation", reply)?,
            errors::NO_PERSON,
        );
    }
    if !["ctx-status", "ctx-wrapup"].iter().all(|name| {
        frozen
            .iter()
            .any(|command| command.command.name == *name && !command.provider.is_empty())
    }) {
        return Err("baseline omitted frozen commands or declaring providers".into());
    }
    let command = if [
        "user_commands_progress_order",
        "user_commands_history_isolation",
    ]
    .contains(&name)
    {
        "ctx-wrapup"
    } else {
        "ctx-status"
    };
    let input = "  untouched input\n20  ";
    let invocation = json!({"session_ref": session_ref, "command": command, "input": input, "invocation_id": case.mint.next("command")});
    let first = expect_ok(
        "user command",
        call(
            &session.route,
            ops::SESSION_USER_COMMAND,
            invocation.clone(),
        )
        .await?,
    )?;
    let answer: UserCommandAnswer = decode("user command", first.clone())?;
    answer.reply.check().map_err(str::to_owned)?;
    if !crate::drive::call_key_well_formed(&answer.call_key) {
        return Err("malformed user-command call_key".into());
    }
    if name == "user_commands_forward_once" {
        let repeat = expect_ok(
            "command repeat",
            call(&session.route, ops::SESSION_USER_COMMAND, invocation).await?,
        )?;
        if repeat != first {
            return Err("repeat changed the command answer".into());
        }
        let observed = case
            .subject
            .user_command_observation(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        if observed.provider_calls.len() != 1
            || observed.provider_calls[0]["name"] != "user_command.invoke"
            || observed.provider_calls[0]["call_key"] != answer.call_key
            || observed.provider_calls[0]["arguments"]["input"] != input
        {
            return Err(format!(
                "expected exactly one verbatim provider call: {:?}",
                observed.provider_calls
            ));
        }
    }
    if name == "user_commands_owner_only" {
        let stranger = case
            .subject
            .session_route(
                case.handle.inner(),
                &session.name,
                &case.subject.stranger_stamp(),
            )
            .await
            .map_err(|e| e.to_string())?;
        for (op, params) in [
            (
                ops::SESSION_USER_COMMAND,
                json!({"session_ref": session_ref, "command": "ctx-status", "input": "", "invocation_id": case.mint.next("stranger")}),
            ),
            (
                ops::SESSION_USER_COMMAND_STATUS,
                json!({"session_ref": session_ref, "call_key": answer.call_key}),
            ),
        ] {
            expect_code(
                op,
                &expect_error(op, call(&stranger, op, params).await?)?,
                errors::SCOPE_OWNER_MISMATCH,
            )?;
        }
    }
    if command == "ctx-wrapup" {
        if answer.reply.state != "accepted" {
            return Err("scripted asynchronous command was not accepted".into());
        }
        let mut terminal = None;
        for _ in 0..15 {
            let status: UserCommandStatus = decode(
                "command status",
                expect_ok(
                    "command status",
                    call(
                        &session.route,
                        ops::SESSION_USER_COMMAND_STATUS,
                        json!({"session_ref": session_ref, "call_key": answer.call_key}),
                    )
                    .await?,
                )?,
            )?;
            status.check().map_err(str::to_owned)?;
            if status.state != "accepted" {
                terminal = Some(status);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
        let status = terminal.ok_or("command fixture did not settle")?;
        if status.state != "done"
            || status.text != "command-private-result"
            || status.progress != ["command-progress-1", "command-progress-2"]
        {
            return Err(format!("status lost result or progress order: {status:?}"));
        }
    } else if answer.reply.state != "done" || answer.reply.text != "command-private-reply" {
        return Err(format!(
            "unexpected scripted command reply: {:?}",
            answer.reply
        ));
    }
    if name == "user_commands_history_isolation" {
        let params = case.params(
            &session,
            "ordinary follow-up",
            &case.mint.next("after-command"),
            None,
        );
        let send: SendReply = decode(
            "follow-up",
            expect_ok(
                "follow-up",
                call(&session.route, ops::SESSION_SEND, params).await?,
            )?,
        )?;
        wait_run_end(case.subject, &session.route, case.declared, None, &send).await?;
        let (_, messages) = read_all(&session.route).await?;
        let observed = case
            .subject
            .user_command_observation(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        if observed.model_inputs.len() < 2 {
            return Err("missing actual follow-up model input".into());
        }
        let history =
            serde_json::to_string(&(messages, observed.model_inputs)).map_err(|e| e.to_string())?;
        for forbidden in [
            "command-private-reply",
            "command-private-result",
            "command-progress-1",
            "command-progress-2",
            "ctx-status",
            "ctx-wrapup",
            "user_command.invoke",
        ] {
            if history.contains(forbidden) {
                return Err(format!(
                    "user-command content leaked into history: {forbidden}"
                ));
            }
        }
    }
    if !fixture.host_replies_outside_history {
        let observed = case
            .subject
            .user_command_observation(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        if !observed.host_commands.is_empty() {
            return Err("unsafe host registered user commands in its menu".into());
        }
    }
    Ok(())
}

/// Kill after the provider has received a user command but before its reply is
/// stored, then verify that restart exposes the same indeterminate outcome
/// through the invocation and status operations without forwarding again.
pub async fn crash_case<S>(
    subject: &S,
    driver: &mut CrashDriver<'_, S>,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let root = work_dir.join("user-command-outcome-unknown");
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let name = mint.next("user-command-crash");
    subject
        .install_script(
            &name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::Text("ordinary model answer".into())]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("follow-up answer".into())]),
                ],
            },
        )
        .await
        .map_err(|error| format!("installing the script of {name}: {error}"))?;
    let fixture = UserCommandFixture {
        collision: false,
        interactive: true,
        host_replies_outside_history: true,
    };
    let owner = subject.owner_stamp();
    let invocation_id = mint.next("user-command-invocation");

    let handle = driver
        .spawn(&root)
        .await
        .map_err(|error| format!("spawning the runner: {error}"))?;
    let route = subject
        .session_route(handle.inner(), &name, &owner)
        .await
        .map_err(|error| format!("opening the route of {name}: {error}"))?;
    // Prepared once the session's route is open, as in every other
    // user-command case: an adapter may compute the session reference from
    // the runner instance the route bound, which does not exist before the
    // spawn.
    let (session_ref, fields) = subject
        .prepare_user_commands(&name, &fixture)
        .await
        .map_err(|error| error.to_string())?;
    let mut invocation = Value::Object(fields);
    invocation["prompt"] = json!("ordinary prompt");
    invocation["send_id"] = json!(mint.next("user-command-send"));
    let admission: SendReply = decode(
        "command-session admission",
        expect_ok(
            "command-session admission",
            call(&route, ops::SESSION_SEND, invocation).await?,
        )?,
    )?;
    wait_run_end(subject, &route, declared, None, &admission).await?;

    let request = json!({
        "session_ref": session_ref,
        "command": "ctx-status",
        "input": "check after restart",
        "invocation_id": invocation_id,
    });
    let trigger: Trigger<'_> = {
        let route = &route;
        let request = request.clone();
        Box::pin(async move {
            let _ = route.request(ops::SESSION_USER_COMMAND, request).await;
        })
    };
    let point = points::USER_COMMAND_FORWARDED;
    driver
        .kill_at(handle, &KillPoint::new(point), trigger)
        .await
        .map_err(|error| format!("killing at {point}: {error}"))?;
    drop(route);

    let handle = driver
        .restart(&root)
        .await
        .map_err(|error| format!("restarting after {point}: {error}"))?;
    let route = subject
        .session_route(handle.inner(), &name, &owner)
        .await
        .map_err(|error| format!("reopening {name}: {error}"))?;
    let answer: UserCommandAnswer = decode(
        "retried command",
        expect_ok(
            "retried command",
            call(&route, ops::SESSION_USER_COMMAND, request).await?,
        )?,
    )?;
    answer.reply.check().map_err(str::to_owned)?;
    if !answer.reply.is_outcome_unknown() || !answer.reply.text.contains("may have run") {
        return Err(format!(
            "lost provider reply must be reported as outcome_unknown with a person-facing warning: {:?}",
            answer.reply
        ));
    }
    let status: UserCommandStatus = decode(
        "command status after restart",
        expect_ok(
            "command status after restart",
            call(
                &route,
                ops::SESSION_USER_COMMAND_STATUS,
                json!({"session_ref":session_ref,"call_key":answer.call_key}),
            )
            .await?,
        )?,
    )?;
    status.check().map_err(str::to_owned)?;
    if !status.is_outcome_unknown()
        || status.text != answer.reply.text
        || !status.progress.is_empty()
    {
        return Err(format!(
            "status did not replay the stored outcome_unknown reply: {status:?}"
        ));
    }

    let observed = subject
        .user_command_observation(&name)
        .await
        .map_err(|error| error.to_string())?;
    if observed.provider_calls.len() != 1
        || observed.provider_calls[0]["name"] != "user_command.invoke"
        || observed.provider_calls[0]["call_key"] != answer.call_key
        || observed.provider_calls[0]["arguments"]["input"] != "check after restart"
    {
        return Err(format!(
            "the lost-reply invocation must reach the provider exactly once: {:?}",
            observed.provider_calls
        ));
    }
    Ok(())
}
