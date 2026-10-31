//! Checks that per-item upfront hints do not change the offered tool set.

use std::collections::BTreeSet;

use cortexkit_role_llm_runner::{capabilities as groups, errors, ops, send::SendReply};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_ok, head, read_all, wait_run_end},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedTurn},
    RunnerRoute, SCRIPTED_TOOL,
};

/// An offered tool not named by the valid hint, to detect accidental filtering.
pub const OTHER_TOOL: &str = "conformance_upfront_other";
/// A name the adapter's fixture item must never offer, even before exclusions.
/// The case relies on that; it does not inspect the catalog.
pub const MISSING_TOOL: &str = "conformance_upfront_missing";

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case
        .checked_describe()
        .await?
        .declares(groups::TOOL_UPFRONT)
    {
        return Ok(Ending::Inapplicable(
            "runner does not declare tool_upfront".into(),
        ));
    }
    let missing: Vec<_> = [Capability::Queue, Capability::TranscriptReads]
        .into_iter()
        .filter(|capability| !case.declared.contains(capability))
        .collect();
    let stale = name == "tool_upfront_stale_refused";
    let hints = if stale {
        vec![MISSING_TOOL.to_owned()]
    } else {
        vec![SCRIPTED_TOOL.to_owned()]
    };
    let variants = if stale {
        vec![Some(hints.as_slice())]
    } else {
        vec![None, Some(hints.as_slice())]
    };
    for hint in variants {
        let session = case
            .open(
                name,
                Script {
                    turns: vec![ScriptedTurn::new(vec![case.mint.text(name).0])],
                },
            )
            .await?;
        let Some(mut fields) = case.subject.upfront_plan_fields(&session.name, hint) else {
            return Ok(Ending::Inapplicable(
                "adapter cannot create plan sessions (upfront_plan_fields)".into(),
            ));
        };
        if !missing.is_empty() {
            return Ok(Ending::Skipped {
                missing,
                reason: "tool_upfront checks send admission and inspect the transcript".into(),
            });
        }
        let items = fields
            .get("plan")
            .and_then(|plan| plan.get("items"))
            .and_then(Value::as_array)
            .ok_or("upfront_plan_fields must return a plan with items")?;
        if items.len() != 1 || items[0].get("upfront") != hint.map(|list| json!(list)).as_ref() {
            return Err(
                "upfront_plan_fields did not apply the requested hint to its single tool item"
                    .into(),
            );
        }
        let before = head(&session.route).await?;
        let durable_before = if stale {
            Some(
                case.subject
                    .upfront_durable_state(case.handle.inner(), &session.name)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        fields.insert("prompt".into(), json!(case.mint.next("upfront-prompt")));
        fields.insert("send_id".into(), json!(case.mint.next("upfront-send")));
        let reply = call(&session.route, ops::SESSION_SEND, Value::Object(fields)).await?;
        if stale {
            let error = expect_error("stale upfront admission", reply)?;
            expect_code("stale upfront admission", &error, errors::PLAN_STALE)?;
            let expected = json!({"kind": "upfront", "provider": case.subject.tool_provider_module(), "name": MISSING_TOOL});
            let differences = error
                .detail
                .as_ref()
                .and_then(|detail| detail.get("differences"))
                .and_then(Value::as_array);
            if !differences.is_some_and(|items| items.contains(&expected)) {
                return Err(format!(
                    "plan_stale must carry the upfront difference {expected}, got {:?}",
                    error.detail
                ));
            }
            let after = head(&session.route).await?;
            let durable_after = case
                .subject
                .upfront_durable_state(case.handle.inner(), &session.name)
                .await
                .map_err(|e| e.to_string())?;
            if durable_before.as_ref() != Some(&durable_after)
                || serde_json::to_value(before).map_err(|e| e.to_string())?
                    != serde_json::to_value(after).map_err(|e| e.to_string())?
                || !read_all(&session.route).await?.1.is_empty()
                || !case
                    .subject
                    .model_requests(&session.name)
                    .await
                    .map_err(|e| e.to_string())?
                    .is_empty()
            {
                return Err("stale upfront admission wrote state or dispatched the model".into());
            }
        } else {
            let reply: SendReply =
                decode("upfront admission", expect_ok("upfront admission", reply)?)?;
            wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
            let requests = case
                .subject
                .model_requests(&session.name)
                .await
                .map_err(|e| e.to_string())?;
            if requests.is_empty() {
                return Err("no model request observed for upfront admission".into());
            }
            let expected: BTreeSet<_> = [SCRIPTED_TOOL.to_owned(), OTHER_TOOL.to_owned()].into();
            for request in requests {
                let actual: BTreeSet<_> = request.tools.iter().cloned().collect();
                if actual != expected || request.tools.len() != expected.len() {
                    return Err(format!(
                        "upfront changed offered tools: expected {expected:?}, got {:?}",
                        request.tools
                    ));
                }
            }
        }
    }
    Ok(Ending::Passed)
}
