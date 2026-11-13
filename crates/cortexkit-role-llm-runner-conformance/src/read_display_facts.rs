//! Read-envelope facts must not become message content or default output.

use crate::{
    cases::{Case, Ending},
    drive::{call, expect_error, expect_field, expect_ok},
    route::RunnerRoute,
    send_helpers::send_and_wait,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    capabilities as groups, display, errors, ops,
    read::{ReadMessage, ReadRequest, ToolTime},
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
    if name == "read_display_facts_created_at" {
        return created_at(case).await;
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
                || entry.get("sender").is_some()
                || entry.get("send_id").is_some()
                || entry.get("tool_times").is_some()
                || entry.get("host_parts").is_some()
                || entry.get("model_fallback").is_some()
                || entry.get("completed_at").is_some()
                || entry.get("created_at").is_some()
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
    let profile = case.subject.serializer_profile(&session.name);
    let mut host_keys_unknown = false;
    let mut fallback_role_unknown = false;
    for (plain, entry) in entries.iter().zip(shown) {
        let read: ReadMessage = serde_json::from_value(entry.clone())
            .map_err(|error| format!("read display facts do not decode: {error}"))?;
        if read.model_fallback.is_some() {
            let projected = profile
                .as_deref()
                .filter(|profile| display::projects(profile))
                .map(|profile| display::project(profile, &read.message, None))
                .transpose()
                .map_err(|error| error.to_string())?;
            match read
                .role
                .as_deref()
                .or_else(|| projected.as_ref().map(|row| row.role.as_str()))
            {
                Some(role) => read.check_model_fallback(role).map_err(str::to_owned)?,
                None => fallback_role_unknown = true,
            }
        }
        if !read.host_parts.is_empty() {
            if read
                .role
                .as_deref()
                .is_some_and(|role| !matches!(role, "user" | "assistant" | "synthetic"))
            {
                return Err(
                    "host_parts are allowed only on a user, assistant or synthetic entry".into(),
                );
            }
            let projected = profile
                .as_deref()
                .filter(|profile| display::projects(profile))
                .map(|profile| display::project(profile, &read.message, None))
                .transpose()
                .map_err(|error| error.to_string())?;
            let role = read
                .role
                .as_deref()
                .or_else(|| projected.as_ref().map(|row| row.role.as_str()));
            let keys = read.served_part_keys().or_else(|| {
                projected
                    .as_ref()
                    .map(|row| row.parts.iter().map(display::DisplayPart::part).collect())
            });
            match (role, keys) {
                (Some(role), Some(keys)) => {
                    read.check_host_parts(role, &keys).map_err(str::to_owned)?
                }
                _ => host_keys_unknown = true,
            }
        }
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
        stripped.as_object_mut().unwrap().remove("host_parts");
        stripped.as_object_mut().unwrap().remove("model_fallback");
        stripped.as_object_mut().unwrap().remove("sender");
        stripped.as_object_mut().unwrap().remove("send_id");
        stripped.as_object_mut().unwrap().remove("completed_at");
        stripped.as_object_mut().unwrap().remove("created_at");
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
    if host_keys_unknown {
        return Ok(Ending::Inapplicable(
            "host_parts key membership needs a known role and stored keys or a shared serializer profile".into(),
        ));
    }
    if fallback_role_unknown {
        return Ok(Ending::Inapplicable(
            "model_fallback validation needs a known role or a shared serializer profile".into(),
        ));
    }
    Ok(Ending::Passed)
}

async fn created_at<S>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let (answer, marker) = case.mint.text("created-assistant");
    let mut session = case
        .open(
            "read-created",
            Script {
                turns: vec![ScriptedTurn::new(vec![answer])],
            },
        )
        .await?;
    let prompt = case.mint.next("creation-user");
    send_and_wait(case, &mut session, None, &prompt, &serde_json::Map::new()).await?;
    let initial = page(&session.route, true).await?;
    let entries = initial["messages"]
        .as_array()
        .ok_or("read page has no messages")?;
    let mut expected = Vec::new();
    for (label, text) in [("user", &prompt), ("assistant", &marker)] {
        let entry = entries
            .iter()
            .find(|entry| entry["message"].to_string().contains(text))
            .ok_or_else(|| format!("creation fixture did not expose the {label} entry"))?;
        let mid = entry["mid"].as_str().ok_or("read entry has no mid")?;
        // Three separated candidates ensure neither of the two other timestamps
        // can be mistaken for the seeded host fact, even near u64 boundaries.
        let created = [120_000_u64, 240_000, 360_000]
            .into_iter()
            .find(|candidate| {
                ["written_at", "completed_at"].iter().all(|field| {
                    entry[*field]
                        .as_u64()
                        .is_none_or(|time| candidate.abs_diff(time) >= 60_000)
                })
            })
            .ok_or("could not separate the seeded creation time from other timestamps")?;
        if !case
            .subject
            .seed_host_created_at(&session.name, mid, created)
            .await
            .map_err(|e| e.to_string())?
        {
            return Ok(Ending::Inapplicable(
                "adapter could not seed a host creation time via seed_host_created_at".into(),
            ));
        }
        expected.push((mid.to_owned(), created));
    }
    let plain = page(&session.route, false).await?;
    if plain["messages"]
        .as_array()
        .ok_or("default read page has no messages")?
        .iter()
        .any(|entry| entry.get("created_at").is_some())
    {
        return Err("default read exposed created_at without opt-in".into());
    }
    let decorated = page(&session.route, true).await?;
    let shown = decorated["messages"]
        .as_array()
        .ok_or("opt-in read page has no messages")?;
    for (mid, created) in expected {
        let entry = shown
            .iter()
            .find(|entry| entry["mid"] == mid)
            .ok_or("creation fixture entry disappeared after seeding")?;
        if entry["created_at"].as_u64() != Some(created) {
            return Err(format!(
                "entry {mid} omitted or changed created_at: expected seeded host time {created}, got {}",
                entry["created_at"]
            ));
        }
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
