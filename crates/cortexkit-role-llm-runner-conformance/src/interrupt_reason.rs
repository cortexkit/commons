//! A planned stop seals an interrupted run with a durable reason (§6.5).

use crate::{
    drive::{call, decode, expect_ok, head, run_result, send_params, Mint},
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    capabilities as groups, ops,
    run::{interrupt_reasons, RunStatus},
    send::SendReply,
};
use serde_json::json;
use std::{collections::BTreeSet, path::Path};

pub async fn shutdown<S: LlmRunnerSubject>(
    subject: &S,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let root = work_dir.join("interrupt-reason-shutdown");
    let handle = subject.spawn(&root).await.map_err(|e| e.to_string())?;
    let session = mint.next("shutdown-session");
    let mut held = mint.call("shutdown-held");
    held.hold = true;
    subject
        .install_script(
            &session,
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::ToolCall(
                    held.clone(),
                )])],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    let route = subject
        .session_route(&handle, &session, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    crate::tags::require_declared(&route, groups::INTERRUPT_REASON, ops::RUN_RESULT).await?;
    let params = send_params(
        subject,
        &session,
        true,
        "shutdown probe",
        &mint.next("shutdown-send"),
        None,
    );
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    subject
        .await_tool_call(&held.arguments)
        .await
        .map_err(|e| e.to_string())?;
    let run = reply.run_id.ok_or("held send has no run_id")?;
    if run_result(&route, &run).await?.state != "active" {
        return Err("shutdown control run is not active".into());
    }
    drop(route);
    // The harness contract stops the module on handle drop. Unlike kill_at,
    // this path asks for an orderly stop rather than simulating process death.
    drop(handle);
    let handle = subject.restart(&root).await.map_err(|e| e.to_string())?;
    let route = subject
        .session_route(&handle, &session, &subject.owner_stamp())
        .await
        .map_err(|e| e.to_string())?;
    let result = run_result(&route, &run).await?;
    if result.state != "interrupted"
        || result.reason.as_deref() != Some(interrupt_reasons::RUNNER_SHUTDOWN)
    {
        return Err(format!(
            "planned stop must record interrupted reason runner_shutdown: {result:?}"
        ));
    }
    let last = head(&route)
        .await?
        .last_run_state
        .ok_or("missing last_run_state after shutdown")?;
    if last.run_id != run || last.reason != result.reason {
        return Err(format!(
            "last_run_state disagrees with durable reason: {last:?}"
        ));
    }
    if declared.contains(&Capability::RunStatus) {
        let status: RunStatus = decode(
            "run.status",
            expect_ok(
                "run.status",
                call(&route, ops::RUN_STATUS, json!({"run_id":run})).await?,
            )?,
        )?;
        if status.state != result.state || status.reason.map(String::from) != result.reason {
            return Err("run.status disagrees with durable shutdown reason".into());
        }
    }
    Ok(())
}
