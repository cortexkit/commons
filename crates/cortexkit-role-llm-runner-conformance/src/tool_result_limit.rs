//! The size check replaces whole output and does not authorize another call.

use crate::{
    cases::{Case, Ending},
    drive::read_all,
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn},
    SCRIPTED_TOOL,
};
use cortexkit_role_llm_runner::errors::tool_result_reasons::TOOL_RESULT_TOO_LARGE;
use serde_json::json;

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let id = case.mint.next("large-output-call");
    let args = json!({"nonce":case.mint.next("large-output-args")});
    let call = ScriptedToolCall {
        tool_call_id: id.clone(),
        tool: SCRIPTED_TOOL.into(),
        arguments: args.clone(),
        result: json!("x".repeat(65_537)),
        hold: false,
    };
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(call)]),
                    ScriptedTurn::new(vec![ScriptedPart::Text("finished".into())]),
                ],
            },
        )
        .await?;
    case.run_prompt(&mut session, "run the scripted call once")
        .await?;
    if name == "tool_result_too_large_not_repeated" {
        let count = case.subject.tool_invocations(&args).await;
        if count != 1 {
            return Err(format!(
                "oversized output caused {count} provider dispatches, expected exactly 1"
            ));
        }
    } else {
        let (_, entries) = read_all(&session.route).await?;
        let observed = entries
            .iter()
            .find_map(|entry| case.subject.oversized_tool_result(&entry.message, &id))
            .ok_or("the native transcript has no observable size error result for the call")?;
        if !observed.is_error
            || observed.reason != TOOL_RESULT_TOO_LARGE
            || observed.bytes != 65_537
            || observed.limit != 65_536
        {
            return Err(format!(
                "oversized output was cut short or lost the size error fields: {observed:?}"
            ));
        }
        let text = observed.text.to_lowercase();
        if !(text.contains("ran") || text.contains("executed"))
            || !text.contains("discard")
            || !text.contains("repeat")
            || !text.contains("not")
            || observed.text.len() > 65_536
        {
            return Err("size error must tell the model the tool ran, its output was discarded, and the call must not be repeated just to see it".into());
        }
    }
    Ok(Ending::Passed)
}
