//! Admission policy is separate from the model-facing plan's application time.

use cortexkit_role_llm_runner::{
    baseline::BaselineReply,
    capabilities as groups,
    delivery_order::PlanSender,
    errors, ops,
    read::ReadRequest,
    rungs,
    send::SendReply,
    session_change::{RefreshReply, RefreshRequest},
};
use serde_json::{json, Map, Value};

use crate::{
    cases::{Case, Ending},
    drive::{
        call, decode, expect_code, expect_error, expect_ok, head, read, read_all, wait_run_end,
    },
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    tags::{send, send_and_wait},
};

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let refresh = name.starts_with("refresh_");
    let required = if refresh {
        vec![
            groups::SESSION_CHANGE,
            groups::DELIVERY_ORDER,
            groups::QUEUE,
            groups::TRANSCRIPT_READS,
        ]
    } else {
        vec![groups::APP_READERS, groups::QUEUE, groups::TRANSCRIPT_READS]
    };
    for group in required {
        if !describe.declares(group) {
            return Ok(Ending::Inapplicable(format!(
                "runner does not declare {group}"
            )));
        }
    }
    if case.subject.head_sender_identities().is_none() {
        return Ok(Ending::Inapplicable(
            "adapter supplies no named-principal routes".into(),
        ));
    }
    match name {
        "refresh_added_sender_admitted_at_acceptance" => {
            sender(case, false, false).await?;
            if case.declared.contains(&Capability::HoldToolCalls) {
                sender(case, false, true).await?;
            }
        }
        "refresh_removed_sender_refused_at_acceptance" => sender(case, true, false).await?,
        "app_readers_listed_reader_admitted"
        | "app_readers_unlisted_reader_refused"
        | "app_readers_reader_cannot_send" => reader(case, name).await?,
        _ => return Err(format!("unknown admission case {name}")),
    }
    Ok(Ending::Passed)
}

async fn snapshot<R: RunnerRoute>(route: &R) -> Result<Value, String> {
    let (pages, _) = read_all(route).await?;
    let baseline = expect_ok(
        "session.baseline",
        call(route, ops::SESSION_BASELINE, json!({})).await?,
    )?;
    Ok(json!({"head": head(route).await?, "pages": pages, "baseline": baseline}))
}

async fn sender<S>(case: &Case<'_, S>, remove: bool, held: bool) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let [_, b] = case.subject.head_sender_identities().unwrap();
    let mut held_call = case.mint.call("admission-held");
    held_call.hold = true;
    let turns = if held {
        vec![
            ScriptedTurn::new(vec![ScriptedPart::ToolCall(held_call.clone())]),
            ScriptedTurn::new(vec![case.mint.text("held-end").0]),
            ScriptedTurn::new(vec![case.mint.text("queued-end").0]),
        ]
    } else {
        vec![
            ScriptedTurn::new(vec![case.mint.text("opening-end").0]),
            ScriptedTurn::new(vec![case.mint.text("sender-end").0]),
        ]
    };
    let mut session = case.open("refresh-sender", Script { turns }).await?;
    let named = json!([PlanSender::new(&b.module).with_idle_start()]);
    let mut members = Map::new();
    members.insert(
        "senders".into(),
        if remove { named.clone() } else { json!([]) },
    );
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
        .ok_or("adapter supplies identities but no plan carrying senders")?;
    let mut plan = fields
        .get("plan")
        .and_then(Value::as_object)
        .cloned()
        .ok_or("adapter's first-send fields contain no plan object")?;
    let opening = send(
        case,
        &mut session,
        Some(&fields),
        "open admission session",
        &Map::new(),
    )
    .await?;
    if held {
        case.subject
            .await_tool_call(&held_call.arguments)
            .await
            .map_err(|e| e.to_string())?;
    } else {
        wait_run_end(case.subject, &session.route, case.declared, None, &opening).await?;
    }
    // Always release a held call, including when the refresh or send check fails.
    let checked = async {
        let baseline: BaselineReply = decode(
            "session.baseline",
            expect_ok(
                "session.baseline",
                call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
            )?,
        )?;
        let baseline = baseline.baseline.ok_or("session.baseline is not ready")?;
        let stated = baseline
            .rungs
            .ok_or("session_change baseline carries no rungs")?;
        let rung = stated
            .tools
            .iter()
            .chain(&stated.system_text)
            .find(|rung| rung.as_str() != rungs::IMMEDIATELY)
            .map(String::as_str)
            .unwrap_or(rungs::IMMEDIATELY);
        plan.insert("senders".into(), if remove { json!([]) } else { named });
        let reply: RefreshReply = decode(
            "session.refresh",
            expect_ok(
                "session.refresh",
                call(
                    &session.route,
                    ops::SESSION_REFRESH,
                    serde_json::to_value(RefreshRequest::new(plan, rung, 1)).unwrap(),
                )
                .await?,
            )?,
        )?;
        if reply.accepted_generation != 1 || reply.run_in_progress != held {
            return Err(format!(
                "refresh did not acknowledge the accepted generation/run state: {reply:?}"
            ));
        }
        let route = case
            .subject
            .session_route(case.handle.inner(), &session.name, &b.stamp)
            .await
            .map_err(|e| e.to_string())?;
        let before = if remove {
            Some(snapshot(&session.route).await?)
        } else {
            None
        };
        let mut params = case.subject.send_fields(&session.name, false);
        params.extend(
            json!({"prompt":case.mint.next("sender-prompt"),
            "send_id":case.mint.next("sender-send"), "delivery":"queue"})
            .as_object()
            .unwrap()
            .clone(),
        );
        let response = call(&route, ops::SESSION_SEND, Value::Object(params)).await?;
        if remove {
            expect_code(
                "removed sender",
                &expect_error("removed sender", response)?,
                errors::SCOPE_OWNER_MISMATCH,
            )?;
            if Some(snapshot(&session.route).await?) != before {
                return Err(
                    "removed sender's refused send changed durable observable state".into(),
                );
            }
        } else {
            let _: SendReply = decode(
                "added sender's queue send",
                expect_ok("added sender's queue send", response)?,
            )?;
        }
        Ok(())
    }
    .await;
    if held {
        let released = case
            .subject
            .release_tool_call(&held_call.arguments)
            .await
            .map_err(|e| e.to_string());
        checked?;
        released?;
    } else {
        checked?;
    }
    Ok(())
}

async fn reader<S>(case: &Case<'_, S>, name: &str) -> Result<(), String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let [a, b] = case.subject.head_sender_identities().unwrap();
    let mut session = case
        .open(
            "app-readers",
            Script {
                turns: vec![ScriptedTurn::new(vec![case.mint.text("reader-answer").0])],
            },
        )
        .await?;
    let mut members = Map::new();
    members.insert("readers".into(), json!([a.module]));
    let fields = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
        .ok_or("adapter supplies identities but no plan carrying readers")?;
    send_and_wait(
        case,
        &mut session,
        Some(&fields),
        "open reader session",
        &Map::new(),
    )
    .await?;
    let identity = if name == "app_readers_unlisted_reader_refused" {
        &b
    } else {
        &a
    };
    let route = case
        .subject
        .session_route(case.handle.inner(), &session.name, &identity.stamp)
        .await
        .map_err(|e| e.to_string())?;
    match name {
        "app_readers_listed_reader_admitted" => {
            head(&route).await?;
            let page = read(&route, &ReadRequest::range(0)).await?;
            if page.messages.is_empty() {
                return Err("listed reader received no entries from the written session".into());
            }
        }
        "app_readers_unlisted_reader_refused" => {
            for (op, params) in [
                (ops::SESSION_HEAD, json!({})),
                (
                    ops::SESSION_READ,
                    serde_json::to_value(ReadRequest::range(0)).unwrap(),
                ),
            ] {
                expect_code(
                    op,
                    &expect_error(op, call(&route, op, params).await?)?,
                    errors::SCOPE_OWNER_MISMATCH,
                )?;
            }
        }
        "app_readers_reader_cannot_send" => {
            let before = snapshot(&session.route).await?;
            let mut params = case.subject.send_fields(&session.name, false);
            params.extend(
                json!({"prompt":"a read grant is not a send grant",
                "send_id":case.mint.next("reader-send"), "delivery":"queue"})
                .as_object()
                .unwrap()
                .clone(),
            );
            expect_code(
                "reader send",
                &expect_error(
                    "reader send",
                    call(&route, ops::SESSION_SEND, Value::Object(params)).await?,
                )?,
                errors::SCOPE_OWNER_MISMATCH,
            )?;
            if snapshot(&session.route).await? != before {
                return Err("reader's refused send changed durable observable state".into());
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}
