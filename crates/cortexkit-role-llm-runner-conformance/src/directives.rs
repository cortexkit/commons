//! Trailing directives are opt-in and only completed runs evaluate them (§9.9).

use cortexkit_role_llm_runner::{
    capabilities as groups, errors, ops, run::RunStatus, send::SendReply,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending, Session},
    drive::{call, decode, expect_error, expect_field, expect_ok, run_result, wait_run_end},
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

async fn send<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    requested: bool,
    max_steps: bool,
) -> Result<SendReply, String>
where
    S::Route: RunnerRoute,
{
    crate::tags::require_declared(&session.route, groups::TURN_DIRECTIVES, "directives").await?;
    let mut params = case.params(
        session,
        "directive probe",
        &case.mint.next("directive-send"),
        None,
    );
    params.as_object_mut().unwrap().remove("directives");
    if requested {
        params["directives"] = json!(["wake"]);
    }
    if max_steps {
        params["stop_when"]["max_steps"] = json!(1);
    }
    decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )
}

async fn check<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    run: &str,
    expected: Value,
) -> Result<(), String>
where
    S::Route: RunnerRoute,
{
    let result = run_result(&session.route, run).await?;
    let actual = serde_json::to_value(&result.directives).unwrap();
    if actual != expected {
        return Err(format!(
            "run.result directives expected {expected}, got {actual} (state {})",
            result.state
        ));
    }
    if case.declared.contains(&Capability::RunStatus) {
        let status: RunStatus = decode(
            "run.status",
            expect_ok(
                "run.status",
                call(&session.route, ops::RUN_STATUS, json!({"run_id":run})).await?,
            )?,
        )?;
        let actual = serde_json::to_value(&status.directives).unwrap();
        if actual != expected {
            return Err(format!(
                "run.status directives expected {expected}, got {actual}"
            ));
        }
    }
    Ok(())
}

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let (text, expected) = match name {
        "turn_directives_in" => ("<wake in=\"90s\"/>", json!([{"name":"wake","in_ms":90000}])),
        "turn_directives_note" => (
            "<wake in=\"3m\">note</wake>",
            json!([{"name":"wake","in_ms":180000,"note":"note"}]),
        ),
        "turn_directives_off" => ("<wake off/>", json!([{"name":"wake","off":true}])),
        "turn_directives_max_steps_reports_none"
        | "turn_directives_cancelled_reports_none"
        | "turn_directives_not_requested_reports_none"
        | "turn_directives_unknown_refused" => ("<wake in=\"90s\"/>", Value::Null),
        _ => return Err(format!("unknown directive case {name}")),
    };
    let cancelled = name == "turn_directives_cancelled_reports_none";
    let limited = name == "turn_directives_max_steps_reports_none";
    let mut held = case.mint.call("directive-held");
    held.hold = cancelled;
    let mut parts = vec![
        ScriptedPart::Reasoning("not final text <wake off/>".into()),
        ScriptedPart::Text("answer ".into()),
    ];
    if cancelled || limited {
        parts.push(ScriptedPart::ToolCall(held.clone()));
    }
    parts.push(ScriptedPart::Text(text.into()));
    let session = case
        .open(
            "directives",
            Script {
                turns: vec![
                    ScriptedTurn::new(parts),
                    ScriptedTurn::new(vec![ScriptedPart::Text("must not run".into())]),
                ],
            },
        )
        .await?;
    if name == "turn_directives_unknown_refused" {
        crate::tags::require_declared(&session.route, groups::TURN_DIRECTIVES, "directives")
            .await?;
        for value in [json!(["unknown"]), json!("wake"), json!(["wake", "wake"])] {
            let mut params = case.params(
                &session,
                "invalid directives",
                &case.mint.next("directive-invalid"),
                None,
            );
            params["directives"] = value;
            let error = expect_error(
                "invalid directives",
                call(&session.route, ops::SESSION_SEND, params).await?,
            )?;
            expect_field(
                "invalid directives",
                &error,
                errors::INVALID_PARAMS,
                "directives",
            )?;
        }
        return Ok(Ending::Passed);
    }
    let reply = send(
        case,
        &session,
        name != "turn_directives_not_requested_reports_none",
        limited,
    )
    .await?;
    if cancelled {
        case.subject
            .await_tool_call(&held.arguments)
            .await
            .map_err(|e| e.to_string())?;
        let run = reply.run_id.as_deref().ok_or("held send has no run_id")?;
        check(case, &session, run, Value::Null).await?;
        let cancel = expect_ok(
            "run.cancel",
            call(&session.route, ops::RUN_CANCEL, json!({"run_id":run})).await?,
        )?;
        if cancel != "ack" {
            return Err(format!("run.cancel answered {cancel}, not ack"));
        }
        case.subject
            .release_tool_call(&held.arguments)
            .await
            .map_err(|e| e.to_string())?;
    }
    let end = wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    let result = run_result(&session.route, &end.run_id).await?;
    let state = if limited {
        "max_steps"
    } else if cancelled {
        "cancelled"
    } else {
        "completed"
    };
    if result.state != state {
        return Err(format!("expected {state}, got {}", result.state));
    }
    check(case, &session, &end.run_id, expected).await?;
    if state == "completed"
        && result
            .final_message
            .as_ref()
            .is_none_or(|message| message.text != format!("answer {text}"))
    {
        return Err("directives changed the stored final_message.text".into());
    }
    Ok(Ending::Passed)
}
