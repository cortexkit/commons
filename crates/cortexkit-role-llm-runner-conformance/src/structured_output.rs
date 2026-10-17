//! Discovery-gated structured-output cases. They observe the scripted model's
//! requests and the scripted tool provider's invocations through the
//! adapter, outside the runner, and everything else over the runner's route.

use std::{collections::BTreeSet, path::Path};

use cortexkit_role_harness::{CrashDriver, KillPoint, Trigger};
use cortexkit_role_llm_runner::{
    errors::{self, tool_result_reasons},
    ops, points,
    run::{RunResult, RunState, RunStatus},
    send::SendReply,
    structured_output::{
        enforcement, reasons, validation, Mode, SkippedSibling, StructuredOutput,
        StructuredOutputResult, SUBMIT_RESULT_TOOL,
    },
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Session},
    drive::{
        call, decode, expect_error, expect_field, expect_ok, head, read_all, run_result,
        wait_run_end, Mint, MAX_POLLS,
    },
    route::RunnerRoute,
    subject::{
        Capability, LlmRunnerSubject, ModelRequestObservation, Script, ScriptedPart,
        ScriptedToolCall, ScriptedTurn,
    },
    SCRIPTED_TOOL,
};

/// The schema every case asks for: an object with a required string.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {"answer": {"type": "string"}},
        "required": ["answer"]
    })
}

fn request(mode: Mode) -> Value {
    serde_json::to_value(StructuredOutput::new(mode, schema()).with_name("conformance_result"))
        .expect("a structured-output request serializes")
}

fn turn(parts: Vec<ScriptedPart>) -> ScriptedTurn {
    ScriptedTurn::new(parts)
}

/// A scripted `submit_result` call carrying `value`. The runner handles it;
/// the scripted tool provider never sees it.
fn submit(tool_call_id: &str, value: Value) -> ScriptedPart {
    ScriptedPart::ToolCall(ScriptedToolCall {
        tool_call_id: tool_call_id.to_owned(),
        tool: SUBMIT_RESULT_TOOL.to_owned(),
        arguments: value,
        result: Value::Null,
        hold: false,
    })
}

/// A part that the runner must never reach: a model request answered with
/// it means the run went on after it should have ended.
fn must_not_start() -> ScriptedTurn {
    turn(vec![ScriptedPart::Text("must not start".into())])
}

/// The params of a send into `session` admitting exactly `tools`.
fn params<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
    first: bool,
    tools: &[&str],
    send_id: &str,
    structured_output: Option<&Value>,
) -> Result<Value, String> {
    let mut params: Map<String, Value> = subject
        .send_fields_with_tools(session, first, tools)
        .ok_or_else(|| {
            format!(
                "the adapter supplies no send fields admitting the tool set {tools:?} \
                 (send_fields_with_tools), which structured_output requires"
            )
        })?;
    params.insert(
        "prompt".into(),
        json!(format!("structured output {send_id}")),
    );
    params.insert("send_id".into(), json!(send_id));
    if let Some(structured_output) = structured_output {
        params.insert("structured_output".into(), structured_output.clone());
    }
    Ok(Value::Object(params))
}

/// Open a fresh session with `script`, its route reporting `supported`.
async fn open<S>(
    case: &Case<'_, S>,
    label: &str,
    script: Script,
    supported: Option<bool>,
) -> Result<Session<S::Route>, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = case.open(label, script).await?;
    case.subject
        .set_structured_output_support(&session.name, supported)
        .await
        .map_err(|e| format!("setting structured-output support of {}: {e}", session.name))?;
    Ok(session)
}

/// Send into `session` and wait for the run to end. Returns `run.result`,
/// after checking that `run.status`, where declared, reports exactly the
/// same `structured_output`.
async fn send<S>(
    case: &Case<'_, S>,
    session: &mut Session<S::Route>,
    tools: &[&str],
    structured_output: Option<&Value>,
) -> Result<RunResult, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let before = head(&session.route).await?.last_run_state.map(|r| r.run_id);
    let send_id = case.mint.next("so-send");
    let first = session.first.is_none();
    let params = params(
        case.subject,
        &session.name,
        first,
        tools,
        &send_id,
        structured_output,
    )?;
    let reply: SendReply = decode(
        "session.send",
        expect_ok(
            "session.send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first.get_or_insert((send_id, String::new()));
    let end = wait_run_end(
        case.subject,
        &session.route,
        case.declared,
        before.as_deref(),
        &reply,
    )
    .await?;
    settled(&session.route, case.declared, &end.run_id).await
}

/// `run.result` of an ended run, agreeing with `run.status` where declared.
async fn settled<R: RunnerRoute>(
    route: &R,
    declared: &BTreeSet<Capability>,
    run_id: &str,
) -> Result<RunResult, String> {
    let result = run_result(route, run_id).await?;
    if declared.contains(&Capability::RunStatus) {
        let status: RunStatus = decode(
            "run.status",
            expect_ok(
                "run.status",
                call(route, ops::RUN_STATUS, json!({ "run_id": run_id })).await?,
            )?,
        )?;
        if status.structured_output != result.structured_output {
            return Err(format!(
                "run.status structured_output {:?} disagrees with run.result {:?}",
                status.structured_output, result.structured_output
            ));
        }
    }
    Ok(result)
}

/// The structured output of a run that ended `completed`, after checking
/// that it keeps the member rules of `StructuredOutputResult::well_formed`.
fn completed(result: &RunResult) -> Result<&StructuredOutputResult, String> {
    if result.run_state() != RunState::Completed {
        return Err(format!("the run ended {}, not completed", result.state));
    }
    let output = result
        .structured_output
        .as_ref()
        .ok_or("the completed run's run.result carries no structured_output")?;
    output
        .well_formed()
        .map_err(|problem| format!("structured_output {output:?} breaks {problem}"))?;
    Ok(output)
}

fn expect(
    output: &StructuredOutputResult,
    mode: &str,
    enforced: Result<(), &str>,
    validated: &str,
) -> Result<(), String> {
    let (wanted_enforcement, wanted_reason) = match enforced {
        Ok(()) => (enforcement::ENFORCED, None),
        Err(reason) => (enforcement::BEST_EFFORT, Some(reason)),
    };
    if output.mode != mode
        || output.enforcement != wanted_enforcement
        || output.reason.as_deref() != wanted_reason
        || output.validation != validated
    {
        return Err(format!(
            "expected {mode} {wanted_enforcement} {wanted_reason:?} {validated}, got {output:?}"
        ));
    }
    Ok(())
}

async fn requests<S: LlmRunnerSubject>(
    subject: &S,
    session: &str,
) -> Result<Vec<ModelRequestObservation>, String> {
    subject
        .model_requests(session)
        .await
        .map_err(|e| format!("observing the model requests of {session}: {e}"))
}

/// The first request offered `submit_result` with an input schema, beside
/// the session's own tools, and asked for no final-text shape. A runner may
/// adapt a schema to its provider, so a schema's presence is checked here
/// and exact bytes only between requests of one run.
fn offers_submit(request: &ModelRequestObservation) -> Result<(), String> {
    if !request.tools.iter().any(|tool| tool == SUBMIT_RESULT_TOOL)
        || !request.tools.iter().any(|tool| tool == SCRIPTED_TOOL)
        || request.submit_result_schema.is_none()
        || request.final_text_schema.is_some()
    {
        return Err(format!(
            "the first model request does not offer {SUBMIT_RESULT_TOOL} with a schema beside \
             {SCRIPTED_TOOL}: {request:?}"
        ));
    }
    Ok(())
}

/// Each listed call has exactly one recorded result saying it never ran,
/// carrying code `skipped_after_submit`: the model called `submit_result` in
/// the same step, so the run ended before the call was dispatched.
async fn never_executed<S>(
    case: &Case<'_, S>,
    session: &Session<S::Route>,
    calls: &[&str],
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (_, messages) = read_all(&session.route).await?;
    for tool_call_id in calls {
        let results: Vec<_> = messages
            .iter()
            .filter_map(|m| {
                case.subject
                    .not_executed_tool_result(&m.message, tool_call_id)
            })
            .collect();
        if results.len() != 1
            || results[0].executed
            || results[0].code != tool_result_reasons::SKIPPED_AFTER_SUBMIT
        {
            return Err(format!(
                "skipped call {tool_call_id} lacks exactly one never-executed \
                 {} result: {results:?}",
                tool_result_reasons::SKIPPED_AFTER_SUBMIT
            ));
        }
    }
    Ok(())
}

/// Each refused send names `field`, writes nothing and starts no model
/// request.
async fn refused<S>(
    case: &Case<'_, S>,
    label: &str,
    tools: &[&str],
    attempts: Vec<(Value, &str)>,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let session = open(
        case,
        label,
        Script {
            turns: vec![must_not_start()],
        },
        Some(true),
    )
    .await?;
    for (structured_output, field) in attempts {
        let params = params(
            case.subject,
            &session.name,
            true,
            tools,
            &case.mint.next("so-refused"),
            Some(&structured_output),
        )?;
        let error = expect_error(
            "structured_output refusal",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?;
        expect_field(
            &format!("structured_output {structured_output}"),
            &error,
            errors::INVALID_PARAMS,
            field,
        )?;
    }
    let (_, messages) = read_all(&session.route).await?;
    if !messages.is_empty() {
        return Err(format!("refused sends wrote {} messages", messages.len()));
    }
    let seen = requests(case.subject, &session.name).await?;
    if !seen.is_empty() {
        return Err(format!("refused sends reached the model: {seen:?}"));
    }
    Ok(())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let answer = json!({ "answer": case.mint.next("answer") });
    match name {
        "structured_output_final_text_enforced_passed" => {
            let text = answer.to_string();
            let mut session = open(
                case,
                "so-final-text",
                Script {
                    turns: vec![turn(vec![ScriptedPart::Text(text)]), must_not_start()],
                },
                Some(true),
            )
            .await?;
            let result = send(case, &mut session, &[], Some(&request(Mode::FinalText))).await?;
            let output = completed(&result)?;
            expect(output, "final_text", Ok(()), validation::PASSED)?;
            if output.value.as_ref() != Some(&answer) {
                return Err(format!("value {:?} is not {answer}", output.value));
            }
            let seen = requests(case.subject, &session.name).await?;
            match seen.as_slice() {
                [only]
                    if only.final_text_schema.is_some()
                        && only.submit_result_schema.is_none()
                        && !only.tools.iter().any(|t| t == SUBMIT_RESULT_TOOL) =>
                {
                    Ok(())
                }
                other => Err(format!(
                    "expected one request shaping the final text and offering no \
                     {SUBMIT_RESULT_TOOL}, got {other:?}"
                )),
            }
        }
        "structured_output_final_text_with_tools_refused" => {
            refused(
                case,
                "so-final-text-tools",
                &[SCRIPTED_TOOL],
                vec![(request(Mode::FinalText), "structured_output.mode")],
            )
            .await
        }
        "structured_output_reserved_name_refused" => {
            refused(
                case,
                "so-reserved",
                &[SCRIPTED_TOOL, SUBMIT_RESULT_TOOL],
                vec![(request(Mode::SubmitTool), "structured_output")],
            )
            .await
        }
        "structured_output_external_ref_refused" => {
            let external = [
                json!({"$ref": "https://example.com/schema.json"}),
                json!({"type": "object", "properties": {"a": {"$ref": "defs.json#/a"}}}),
                json!({"$dynamicRef": "https://example.com/meta.json#meta"}),
            ];
            let attempts = external
                .into_iter()
                .map(|schema| {
                    let request = StructuredOutput::new(Mode::SubmitTool, schema);
                    (
                        serde_json::to_value(request).expect("serializes"),
                        "structured_output.schema",
                    )
                })
                .collect();
            refused(case, "so-external-ref", &[SCRIPTED_TOOL], attempts).await
        }
        "structured_output_submit_tool_ends_run" => {
            let sibling = case.mint.call("so-sibling");
            let mut session = open(
                case,
                "so-submit",
                Script {
                    turns: vec![
                        turn(vec![
                            case.mint.text("before submitting").0,
                            submit("so-submit-call", answer.clone()),
                            ScriptedPart::ToolCall(sibling.clone()),
                        ]),
                        must_not_start(),
                    ],
                },
                Some(true),
            )
            .await?;
            let result = send(
                case,
                &mut session,
                &[SCRIPTED_TOOL],
                Some(&request(Mode::SubmitTool)),
            )
            .await?;
            let output = completed(&result)?;
            expect(output, "submit_tool", Ok(()), validation::PASSED)?;
            if output.value.as_ref() != Some(&answer) {
                return Err(format!("value {:?} is not {answer}", output.value));
            }
            let listed = vec![SkippedSibling::new(&sibling.tool_call_id, SCRIPTED_TOOL)];
            if output.skipped_siblings.as_ref() != Some(&listed) {
                return Err(format!(
                    "skipped_siblings {:?} is not {listed:?}",
                    output.skipped_siblings
                ));
            }
            let seen = requests(case.subject, &session.name).await?;
            match seen.as_slice() {
                [first] => offers_submit(first)?,
                other => {
                    return Err(format!(
                        "submit_result did not end the run after its step: {} model \
                         requests: {other:?}",
                        other.len()
                    ))
                }
            }
            never_executed(case, &session, &[&sibling.tool_call_id]).await
        }
        "structured_output_sibling_never_dispatched" => {
            let sibling = case.mint.call("so-early-sibling");
            let second = json!({ "answer": case.mint.next("second") });
            let mut session = open(
                case,
                "so-sibling",
                Script {
                    turns: vec![
                        turn(vec![
                            ScriptedPart::ToolCall(sibling.clone()),
                            submit("so-first-submit", answer.clone()),
                            submit("so-second-submit", second),
                        ]),
                        must_not_start(),
                    ],
                },
                Some(true),
            )
            .await?;
            let result = send(
                case,
                &mut session,
                &[SCRIPTED_TOOL],
                Some(&request(Mode::SubmitTool)),
            )
            .await?;
            let invoked = case.subject.tool_invocations(&sibling.arguments).await;
            if invoked != 0 {
                return Err(format!(
                    "the sibling {} emitted before submit_result was invoked {invoked} \
                     times at the scripted tool provider",
                    sibling.tool_call_id
                ));
            }
            let output = completed(&result)?;
            if output.value.as_ref() != Some(&answer) {
                return Err(format!(
                    "value {:?} is not the first submit_result's {answer}",
                    output.value
                ));
            }
            let listed = vec![
                SkippedSibling::new(&sibling.tool_call_id, SCRIPTED_TOOL),
                SkippedSibling::new("so-second-submit", SUBMIT_RESULT_TOOL),
            ];
            if output.skipped_siblings.as_ref() != Some(&listed) {
                return Err(format!(
                    "skipped_siblings {:?} is not {listed:?}",
                    output.skipped_siblings
                ));
            }
            never_executed(case, &session, &[&sibling.tool_call_id, "so-second-submit"]).await
        }
        "structured_output_absent_without_submit" => {
            let mut session = open(
                case,
                "so-absent",
                Script {
                    turns: vec![
                        turn(vec![ScriptedPart::Text(
                            "finished without submitting".into(),
                        )]),
                        must_not_start(),
                    ],
                },
                None,
            )
            .await?;
            let result = send(
                case,
                &mut session,
                &[SCRIPTED_TOOL],
                Some(&request(Mode::SubmitTool)),
            )
            .await?;
            let output = completed(&result)?;
            expect(
                output,
                "submit_tool",
                Err(reasons::CATALOG_UNKNOWN),
                validation::ABSENT,
            )?;
            if output.value.is_some() || output.skipped_siblings.as_deref() != Some(&[]) {
                return Err(format!(
                    "an absent result carries a value or a non-empty skipped_siblings: {output:?}"
                ));
            }
            Ok(())
        }
        "structured_output_failed_validation_completes" => {
            let wrong = json!({ "answer": 42 });
            let mut session = open(
                case,
                "so-failed",
                Script {
                    turns: vec![
                        turn(vec![submit("so-wrong-submit", wrong.clone())]),
                        must_not_start(),
                    ],
                },
                Some(false),
            )
            .await?;
            let result = send(
                case,
                &mut session,
                &[SCRIPTED_TOOL],
                Some(&request(Mode::SubmitTool)),
            )
            .await?;
            let output = completed(&result)?;
            expect(
                output,
                "submit_tool",
                Err(reasons::ROUTE_UNSUPPORTED),
                validation::FAILED,
            )?;
            if output.value.as_ref() != Some(&wrong) {
                return Err(format!(
                    "a failed value that parsed must be reported, got {:?}",
                    output.value
                ));
            }
            Ok(())
        }
        "structured_output_not_inherited" => {
            let mut session = open(
                case,
                "so-not-inherited",
                Script {
                    turns: vec![
                        turn(vec![submit("so-inherit-submit", answer.clone())]),
                        turn(vec![ScriptedPart::Text("a plain answer".into())]),
                        must_not_start(),
                    ],
                },
                Some(true),
            )
            .await?;
            let first = send(
                case,
                &mut session,
                &[SCRIPTED_TOOL],
                Some(&request(Mode::SubmitTool)),
            )
            .await?;
            expect(
                completed(&first)?,
                "submit_tool",
                Ok(()),
                validation::PASSED,
            )?;
            let second = send(case, &mut session, &[SCRIPTED_TOOL], None).await?;
            if second.run_state() != RunState::Completed || second.structured_output.is_some() {
                return Err(format!(
                    "a run sent without structured_output must complete without one: {second:?}"
                ));
            }
            let seen = requests(case.subject, &session.name).await?;
            match seen.as_slice() {
                [with, without] => {
                    offers_submit(with)?;
                    if without.tools.iter().any(|t| t == SUBMIT_RESULT_TOOL)
                        || without.submit_result_schema.is_some()
                        || without.final_text_schema.is_some()
                    {
                        return Err(format!(
                            "the second run inherited the first run's schema: {without:?}"
                        ));
                    }
                    Ok(())
                }
                other => Err(format!("expected two model requests, got {other:?}")),
            }
        }
        other => Err(format!("unknown structured-output case {other}")),
    }
}

/// Kill the runner right after a step's tool result is durable, restart it
/// on the same root, and check that the run's next model request carries the
/// same tools and the same schema bytes as its first. A runner that resumes
/// the cut run itself is checked on that run; one that seals it
/// `interrupted` is checked on the owner's follow-up send, which repeats the
/// same `structured_output`.
///
/// The suite's crash driver kills the runner at each durable point at most
/// once per run, and the general crash cases already use the point after a
/// tool result is saved. So this case uses a separate driver on a separate
/// state root to kill at that point again. The suite's count of real kills,
/// which other cases check, does not include this one.
pub async fn restart<S>(
    subject: &S,
    work_dir: &Path,
    declared: &BTreeSet<Capability>,
    mint: &Mint,
) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let root = work_dir.join("structured-output-restart");
    std::fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let name = mint.next("so-restart");
    let echo = mint.call("so-restart-echo");
    let answer = json!({ "answer": mint.next("restart-answer") });
    subject
        .install_script(
            &name,
            Script {
                turns: vec![
                    turn(vec![ScriptedPart::ToolCall(echo)]),
                    turn(vec![submit("so-restart-submit", answer.clone())]),
                    must_not_start(),
                ],
            },
        )
        .await
        .map_err(|e| format!("installing the script of {name}: {e}"))?;
    subject
        .set_structured_output_support(&name, Some(true))
        .await
        .map_err(|e| format!("setting structured-output support of {name}: {e}"))?;
    let structured_output = request(Mode::SubmitTool);
    let first = params(
        subject,
        &name,
        true,
        &[SCRIPTED_TOOL],
        &mint.next("so-restart-send"),
        Some(&structured_output),
    )?;
    let owner = subject.owner_stamp();
    let mut driver = CrashDriver::new(subject);
    let handle = driver
        .spawn(&root)
        .await
        .map_err(|e| format!("spawning: {e}"))?;
    let route = subject
        .session_route(handle.inner(), &name, &owner)
        .await
        .map_err(|e| format!("opening the route of {name}: {e}"))?;
    let trigger: Trigger<'_> = {
        let route = &route;
        let params = first.clone();
        Box::pin(async move {
            if route.request(ops::SESSION_SEND, params).await.is_err() {
                return;
            }
            for _ in 0..MAX_POLLS {
                match head(route).await {
                    Err(_) => return,
                    Ok(head)
                        if head
                            .last_run_state
                            .as_ref()
                            .is_some_and(|last| RunState::parse(&last.state).is_terminal()) =>
                    {
                        return
                    }
                    Ok(_) => subject.pause().await,
                }
            }
        })
    };
    let point = points::TOOL_RESULT_RECORDED;
    driver
        .kill_at(handle, &KillPoint::new(point), trigger)
        .await
        .map_err(|e| format!("killing at {point}: {e}"))?;
    drop(route);
    let handle = driver
        .restart(&root)
        .await
        .map_err(|e| format!("restarting after {point}: {e}"))?;
    let route = subject
        .session_route(handle.inner(), &name, &owner)
        .await
        .map_err(|e| format!("reopening {name}: {e}"))?;
    let retry: SendReply = decode(
        "the retried send",
        expect_ok(
            "the first send, retried after the restart",
            call(&route, ops::SESSION_SEND, first).await?,
        )?,
    )?;
    let end = wait_run_end(subject, &route, declared, None, &retry).await?;
    let mut result = settled(&route, declared, &end.run_id).await?;
    if result.run_state() == RunState::Interrupted {
        let follow_up = params(
            subject,
            &name,
            false,
            &[SCRIPTED_TOOL],
            &mint.next("so-restart-follow-up"),
            Some(&structured_output),
        )?;
        let reply: SendReply = decode(
            "the follow-up send",
            expect_ok(
                "the follow-up send",
                call(&route, ops::SESSION_SEND, follow_up).await?,
            )?,
        )?;
        let end = wait_run_end(subject, &route, declared, Some(&end.run_id), &reply).await?;
        result = settled(&route, declared, &end.run_id).await?;
    }
    let output = completed(&result)?;
    if output.validation != validation::PASSED || output.value.as_ref() != Some(&answer) {
        return Err(format!(
            "the run after the restart did not submit {answer}: {output:?}"
        ));
    }
    let seen = requests(subject, &name).await?;
    match seen.as_slice() {
        [first, .., last] if first == last => offers_submit(first),
        [first, .., last] => Err(format!(
            "the model request after the restart differs from the first: {first:?} then {last:?}"
        )),
        other => Err(format!(
            "expected a model request before and after the restart, got {other:?}"
        )),
    }
}
