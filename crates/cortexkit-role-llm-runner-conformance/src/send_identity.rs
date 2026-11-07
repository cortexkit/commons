//! Module-send identities stay in read envelopes and match the ids callers actually saw.

use cortexkit_role_llm_runner::{
    capabilities as groups,
    delivery_order::{Mode, PlanSender},
    errors,
    history::HistoryRequest,
    ops,
    read::ReadRequest,
    send::SendReply,
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_ok, wait_run_end},
    route::{Reply, RunnerRoute},
    send_helpers::send_and_wait,
    subject::{LlmRunnerSubject, Script, ScriptedTurn},
};

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let required = name == "read_display_facts_send_id";
    if describe.declares(groups::SEND_IDS) != required {
        return Ok(Ending::Inapplicable(if required {
            "runner does not declare send_ids".into()
        } else {
            "send_ids guarantee is checked by read_display_facts_send_id".into()
        }));
    }
    for group in [
        groups::MESSAGE_SENDERS,
        groups::READ_DISPLAY_FACTS,
        groups::DELIVERY_ORDER,
    ] {
        if !describe.declares(group) {
            return Ok(Ending::Inapplicable(format!(
                "runner does not declare {group}"
            )));
        }
    }
    if !Mode::IdleStart.is_declared_by(&describe) {
        return Ok(Ending::Inapplicable(
            "runner does not declare idle_start".into(),
        ));
    }
    let Some([_, sender]) = case.subject.head_sender_identities() else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no named-principal routes (head_sender_identities)".into(),
        ));
    };
    let mut session = case
        .open(
            "send-identity",
            Script {
                turns: (0..3)
                    .map(|_| ScriptedTurn::new(vec![case.mint.text("send-identity-answer").0]))
                    .collect(),
            },
        )
        .await?;
    let members = json!({"senders":[PlanSender::new(&sender.module).with_idle_start()]});
    let Some(fields) =
        case.subject
            .send_fields_with_plan(&session.name, true, members.as_object().unwrap())
    else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no plan carrying senders".into(),
        ));
    };
    send_and_wait(
        case,
        &mut session,
        Some(&fields),
        &case.mint.next("send-identity-opening"),
        &Map::new(),
    )
    .await?;
    let route = case
        .subject
        .session_route(case.handle.inner(), &session.name, &sender.stamp)
        .await
        .map_err(|error| error.to_string())?;
    let prompt = case.mint.next("keyed-prompt");
    let send_id = case.mint.next("keyed-send");
    let mut params = case.subject.send_fields(&session.name, false);
    params.extend(
        json!({"prompt":prompt,"send_id":send_id,"delivery":["idle_start"]})
            .as_object()
            .unwrap()
            .clone(),
    );
    let reply: SendReply = decode(
        "keyed identity send",
        expect_ok(
            "keyed identity send",
            call(&route, ops::SESSION_SEND, Value::Object(params)).await?,
        )?,
    )?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    check_entry(&session.route, &prompt, Some(&send_id), required).await?;

    let prompt = case.mint.next("unkeyed-prompt");
    let mut params = case.subject.send_fields(&session.name, false);
    params.remove("send_id");
    params.extend(
        json!({"prompt":prompt,"delivery":["idle_start"]})
            .as_object()
            .unwrap()
            .clone(),
    );
    let response = call(&route, ops::SESSION_SEND, Value::Object(params)).await?;
    if matches!(&response, Reply::Error(error) if error.code == errors::SCOPE_OWNER_MISMATCH) {
        return Ok(Ending::PassedWithInapplicableRule(
            "read_display_facts_send_id_unkeyed: runner does not admit unkeyed non-owner sends"
                .into(),
        ));
    }
    let reply: SendReply = decode(
        "unkeyed identity send",
        expect_ok("unkeyed identity send", response)?,
    )?;
    wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
    check_entry(
        &session.route,
        &prompt,
        reply.submission_id.as_deref(),
        required,
    )
    .await?;
    if reply.submission_id.is_none() {
        return Ok(Ending::PassedWithInapplicableRule("read_display_facts_send_id_unkeyed: runner returned no submission_id; verified absence instead".into()));
    }
    Ok(Ending::Passed)
}

async fn check_entry<R: RunnerRoute>(
    route: &R,
    prompt: &str,
    expected: Option<&str>,
    required: bool,
) -> Result<(), String> {
    let read = |include| {
        serde_json::to_value(
            ReadRequest::range(0)
                .with_limit(100)
                .with_include_display_facts(include),
        )
        .unwrap()
    };
    let plain = expect_ok(
        "default identity read",
        call(route, ops::SESSION_READ, read(false)).await?,
    )?;
    let decorated = expect_ok(
        "opted-in identity read",
        call(route, ops::SESSION_READ, read(true)).await?,
    )?;
    if plain["history_version"] != decorated["history_version"] {
        return Err("send_id display fact changed history_version".into());
    }
    let entries = plain["messages"]
        .as_array()
        .ok_or("default read has no messages")?;
    if entries.iter().any(|entry| entry.get("send_id").is_some()) {
        return Err("default read exposed send_id without opt-in".into());
    }
    let plain = entries
        .iter()
        .find(|entry| entry["message"].to_string().contains(prompt))
        .ok_or("module send missing from default read")?;
    let entries = decorated["messages"]
        .as_array()
        .ok_or("opted-in read has no messages")?;
    if entries
        .iter()
        .any(|entry| entry["role"] == "assistant" && entry.get("send_id").is_some())
    {
        return Err("model reply carries a module send_id".into());
    }
    let entry = entries
        .iter()
        .find(|entry| entry["mid"] == plain["mid"])
        .ok_or("module send missing from opted-in read")?;
    check_id(entry, expected, required, "read")?;
    for field in ["message", "digest", "part_digests"] {
        if entry[field] != plain[field] {
            return Err(format!("send_id display fact changed {field}"));
        }
    }
    let request =
        HistoryRequest::from_mid(plain["mid"].as_str().ok_or("read mid is not a string")?)
            .with_limit(1);
    let history = expect_ok(
        "raw identity history",
        call(
            route,
            ops::SESSION_HISTORY,
            serde_json::to_value(request).unwrap(),
        )
        .await?,
    )?;
    let entry = history["messages"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["mid"] == plain["mid"]))
        .ok_or("module send missing from raw history")?;
    check_id(entry, expected, required, "history")?;
    if entry["message"] != plain["message"] {
        return Err("send_id changed raw history message bytes".into());
    }
    Ok(())
}

fn check_id(entry: &Value, expected: Option<&str>, required: bool, op: &str) -> Result<(), String> {
    if !required && entry.get("send_id").is_none() {
        return Ok(());
    }
    if entry.get("send_id").and_then(Value::as_str) != expected
        || (expected.is_none() && entry.get("send_id").is_some())
    {
        return Err(format!(
            "{op} entry send_id is {:?}, expected {expected:?}",
            entry.get("send_id")
        ));
    }
    Ok(())
}
