//! Read-envelope facts must not become message content or default output.

use crate::{
    cases::{Case, Ending},
    drive::{call, expect_error, expect_field, expect_ok},
    route::RunnerRoute,
    send_helpers::send_and_wait,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    capabilities as groups, errors, ops,
    read::{ReadRequest, ToolTime},
};
use serde_json::{json, Value};

async fn page<R: RunnerRoute>(route: &R, include: bool) -> Result<Value, String> {
    expect_ok(
        "session.read",
        call(
            route,
            ops::SESSION_READ,
            serde_json::to_value(
                ReadRequest::range(0)
                    .with_limit(100)
                    .with_include_display_facts(include),
            )
            .unwrap(),
        )
        .await?,
    )
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let negative = name == "read_display_facts_undeclared_refused";
    if describe.declares(groups::READ_DISPLAY_FACTS) == negative {
        return Ok(Ending::Inapplicable(
            "describe groups do not support this question".into(),
        ));
    }
    if name == "read_display_facts_completed_at" {
        return completed_at(case).await;
    }
    let calls: Vec<_> = (0..3)
        .map(|index| case.mint.call(&format!("display-call-{index}")))
        .collect();
    let mut session = case
        .open(
            "read-facts",
            Script {
                turns: vec![
                    ScriptedTurn::new(calls.iter().cloned().map(ScriptedPart::ToolCall).collect()),
                    ScriptedTurn::new(vec![case.mint.text("done").0]),
                ],
            },
        )
        .await?;
    if negative {
        let error = expect_error(
            "undeclared display facts",
            call(
                &session.route,
                ops::SESSION_READ,
                json!({"include_display_facts":true}),
            )
            .await?,
        )?;
        expect_field(
            "undeclared display facts",
            &error,
            errors::INVALID_PARAMS,
            "include_display_facts",
        )?;
        return Ok(Ending::Passed);
    }
    let expected = vec![
        ToolTime::new(&calls[0].tool_call_id)
            .with_started_at(100)
            .with_finished_at(150),
        ToolTime::new(&calls[1].tool_call_id)
            .with_started_at(101)
            .with_unanswered("future_reason"),
        ToolTime::new(&calls[2].tool_call_id),
    ];
    let prompt = case.mint.next("client-prompt");
    let client_id = "phone_read_facts_1";
    let extra = json!({"client_message_id":client_id})
        .as_object()
        .unwrap()
        .clone();
    send_and_wait(case, &mut session, None, &prompt, &extra).await?;
    if !case
        .subject
        .seed_read_tool_times(&session.name, &expected)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Ending::Inapplicable(
            "adapter cannot seed stored read tool times".into(),
        ));
    }
    let default = page(&session.route, false).await?;
    let entries = default["messages"]
        .as_array()
        .ok_or("read page has no messages array")?;
    if name == "read_display_facts_default_absent" {
        if entries.iter().any(|entry| {
            entry.get("client_message_id").is_some()
                || entry.get("tool_times").is_some()
                || entry.get("completed_at").is_some()
                || entry.get("reasoning_summaries").is_some()
        }) {
            return Err("default read exposed display facts without opt-in".into());
        }
        return Ok(Ending::Passed);
    }
    let decorated = page(&session.route, true).await?;
    let shown = decorated["messages"]
        .as_array()
        .ok_or("opt-in read has no messages array")?;
    if entries.len() != shown.len() {
        return Err("display facts changed the read's message count".into());
    }
    let mut user_found = false;
    let mut calls_found = false;
    for (plain, entry) in entries.iter().zip(shown) {
        if plain["mid"] != entry["mid"]
            || serde_json::to_vec(&plain["message"]).unwrap()
                != serde_json::to_vec(&entry["message"]).unwrap()
        {
            return Err("display facts changed mid or message bytes".into());
        }
        if plain["message"].to_string().contains(&prompt) {
            user_found = true;
            if entry["client_message_id"] != client_id {
                return Err("sent user entry omitted or changed client_message_id".into());
            }
            if entry.get("tool_times").is_some() {
                return Err("user entry with no calls carried tool_times".into());
            }
        } else if entry.get("client_message_id").is_some() {
            return Err("client id appeared on a different entry".into());
        }
        if calls
            .iter()
            .all(|call| plain["message"].to_string().contains(&call.tool_call_id))
        {
            calls_found = true;
            if entry["tool_times"] != serde_json::to_value(&expected).unwrap() {
                return Err(
                    "tool_times missing, reordered, invented or different from stored facts".into(),
                );
            }
        }
        let mut stripped = entry.clone();
        stripped
            .as_object_mut()
            .ok_or("read entry is not an object")?
            .remove("client_message_id");
        stripped.as_object_mut().unwrap().remove("tool_times");
        stripped.as_object_mut().unwrap().remove("completed_at");
        stripped
            .as_object_mut()
            .unwrap()
            .remove("reasoning_summaries");
        if &stripped != plain {
            return Err("opt-in changed other envelope members".into());
        }
    }
    if !user_found || !calls_found {
        return Err("fixture did not expose the sent user message and call batch".into());
    }
    Ok(Ending::Passed)
}

async fn completed_at<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if !case.subject.reports_read_completed_at() {
        return Ok(Ending::Inapplicable(
            "adapter cannot report host assistant finishes".into(),
        ));
    }
    let (answer, marker) = case.mint.text("finished-assistant");
    let mut session = case
        .open(
            "read-completed",
            Script {
                turns: vec![ScriptedTurn::new(vec![answer])],
            },
        )
        .await?;
    let prompt = case.mint.next("completion-user");
    send_and_wait(case, &mut session, None, &prompt, &serde_json::Map::new()).await?;
    let decorated = page(&session.route, true).await?;
    let mut assistant = false;
    let mut user = false;
    for entry in decorated["messages"]
        .as_array()
        .ok_or("read page has no messages")?
    {
        let body = entry["message"].to_string();
        if body.contains(&marker) {
            assistant = true;
            entry["completed_at"]
                .as_u64()
                .ok_or("finished assistant entry omitted completed_at")?;
        }
        if body.contains(&prompt) || entry["role"] == "user" {
            user = true;
            if entry.get("completed_at").is_some() {
                return Err("user entry carried completed_at".into());
            }
        }
    }
    if !assistant || !user {
        return Err("completion fixture did not expose both assistant and user entries".into());
    }
    Ok(Ending::Passed)
}
