//! Keyed retry isolation across a change of attested sender.

use crate::{
    cases::{Case, Ending},
    drive::{
        call, decode, expect_code, expect_error, expect_field, expect_ok, read, read_all,
        wait_run_end,
    },
    route::RunnerRoute,
    send_helpers::{params, send_and_wait},
    subject::{Capability, LlmRunnerSubject, Script, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    capabilities as groups,
    delivery_order::{DeliveryOrder, Mode, PlanSender},
    errors, ops,
    read::ReadRequest,
    send::SendReply,
    session_change::RefreshRequest,
};
use serde_json::{json, Map, Value};

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let negative = name == "sender_succession_undeclared_refused";
    let deadline = name == "delivery_deadline_retry_keeps_original_expiry";
    if !describe.declares(groups::DELIVERY_ORDER)
        || (!negative && !deadline && !describe.declares(groups::SENDER_SUCCESSION))
        || (negative && describe.declares(groups::SENDER_SUCCESSION))
    {
        return Ok(Ending::Inapplicable(
            "describe groups do not support this question".into(),
        ));
    }
    if deadline {
        let Some(mode) = [Mode::ToolResult, Mode::Steer]
            .into_iter()
            .find(|mode| mode.is_declared_by(&describe))
        else {
            return Ok(Ending::Inapplicable(
                "runner offers no non-starting list mode for the expiry fixture".into(),
            ));
        };
        return deadline_retry(case, mode).await;
    }
    if !negative && !Mode::IdleStart.is_declared_by(&describe) {
        return Ok(Ending::Inapplicable(
            "runner offers no idle_start list mode for the sender fixture".into(),
        ));
    }
    let Some([a, b]) = case.subject.head_sender_identities() else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no two named-principal routes".into(),
        ));
    };
    if a.module == b.module
        || a.stamp == b.stamp
        || a.stamp == case.subject.owner_stamp()
        || b.stamp == case.subject.owner_stamp()
    {
        return Err("sender fixture must supply two distinct non-owner identities".into());
    }
    let mut session = case
        .open(
            "succession",
            Script {
                turns: (0..5)
                    .map(|_| ScriptedTurn::new(vec![case.mint.text("answer").0]))
                    .collect(),
            },
        )
        .await?;
    let mut members = Map::new();
    let successor = PlanSender::new(&b.module)
        .with_idle_start()
        .with_succeeds(&a.module);
    members.insert(
        "senders".into(),
        json!([
            PlanSender::new(&a.module).with_idle_start(),
            successor.clone()
        ]),
    );
    let Some(fields) = case
        .subject
        .send_fields_with_plan(&session.name, true, &members)
    else {
        return Ok(Ending::Inapplicable(
            "adapter supplies no plan carrying senders".into(),
        ));
    };
    if negative {
        let before = read(&session.route, &ReadRequest::tail()).await?;
        let error = expect_error(
            "undeclared succession",
            call(
                &session.route,
                ops::SESSION_SEND,
                params(
                    case,
                    &session,
                    Some(&fields),
                    "must not write",
                    &case.mint.next("send"),
                    &Map::new(),
                ),
            )
            .await?,
        )?;
        expect_code("undeclared succession", &error, errors::INVALID_PARAMS)?;
        expect_field(
            "undeclared succession",
            &error,
            errors::INVALID_PARAMS,
            "plan.senders[1].succeeds",
        )?;
        if read(&session.route, &ReadRequest::tail()).await? != before {
            return Err("refused succession plan wrote a transcript entry".into());
        }
        return Ok(Ending::Passed);
    }
    send_and_wait(case, &mut session, Some(&fields), "opening", &Map::new()).await?;
    let route_a = case
        .subject
        .session_route(case.handle.inner(), &session.name, &a.stamp)
        .await
        .map_err(|e| e.to_string())?;
    let route_b = case
        .subject
        .session_route(case.handle.inner(), &session.name, &b.stamp)
        .await
        .map_err(|e| e.to_string())?;
    let marker = case.mint.next("payload");
    let id = case.mint.next("send-id");
    let order = DeliveryOrder::new(vec![Mode::IdleStart]);
    let original_params = params(case, &session, None, &marker, &id, &order.to_send_members());
    let original_route = &route_a;
    let first: SendReply = decode(
        "original send",
        expect_ok(
            "original send",
            call(original_route, ops::SESSION_SEND, original_params.clone()).await?,
        )?,
    )?;
    wait_run_end(case.subject, &session.route, case.declared, None, &first).await?;
    let receipt = expect_ok(
        "original receipt",
        call(original_route, ops::SESSION_SEND, original_params.clone()).await?,
    )?;
    // A durably accepted refresh changes the sender list before the next run.
    // The successor must still find the removed predecessor's send while the
    // rest of the refreshed plan waits to apply.
    if case.declared.contains(&Capability::SessionChange) {
        let mut plan = fields
            .get("plan")
            .and_then(Value::as_object)
            .cloned()
            .ok_or("fixture plan missing")?;
        plan.insert("senders".into(), json!([successor]));
        expect_ok(
            "remove predecessor",
            call(
                &session.route,
                ops::SESSION_REFRESH,
                serde_json::to_value(RefreshRequest::new(plan, "immediately", 1)).unwrap(),
            )
            .await?,
        )?;
    }
    let before = read_all(&session.route).await?.1;
    match name {
        "sender_succession_same_payload_receipt" => {
            let reply = expect_ok(
                "successor retry",
                call(&route_b, ops::SESSION_SEND, original_params).await?,
            )?;
            if reply != receipt {
                return Err(format!(
                    "successor got a different receipt: {reply:?} != {receipt:?}"
                ));
            }
            if read_all(&session.route).await?.1 != before {
                return Err("successor retry wrote another message".into());
            }
        }
        "sender_succession_changed_payload_refused" => {
            let mut changed = original_params;
            changed["prompt"] = json!(case.mint.next("different"));
            let error = expect_error(
                "successor conflict",
                call(&route_b, ops::SESSION_SEND, changed).await?,
            )?;
            expect_code("successor conflict", &error, errors::SEND_ID_REUSE)?;
            if read_all(&session.route).await?.1 != before {
                return Err("conflicting retry wrote a message".into());
            }
        }
        "sender_succession_third_sender_fresh" => {
            let reply = expect_ok(
                "third sender",
                call(&session.route, ops::SESSION_SEND, original_params).await?,
            )?;
            let sent: SendReply = decode("third sender", reply.clone())?;
            wait_run_end(case.subject, &session.route, case.declared, None, &sent).await?;
            if reply == receipt {
                return Err("third sender got the predecessor's receipt".into());
            }
            if read_all(&session.route)
                .await?
                .1
                .iter()
                .filter(|entry| entry.message.to_string().contains(&marker))
                .count()
                != 2
            {
                return Err("third sender did not get a fresh message".into());
            }
        }
        "sender_succession_predecessor_status" => {
            let reply = expect_ok(
                "successor status",
                call(&route_b, ops::SEND_STATUS, json!({"send_id":id})).await?,
            )?;
            if reply != receipt {
                return Err("successor status differs from predecessor receipt".into());
            }
            if read_all(&session.route).await?.1 != before {
                return Err("status changed transcript".into());
            }
        }
        _ => return Err(format!("unknown succession case {name}")),
    }
    Ok(Ending::Passed)
}

async fn deadline_retry<S>(case: &Case<'_, S>, mode: Mode) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if case.subject.delivery_clock_now_ms().is_none()
        || !case.declared.contains(&Capability::RetentionClock)
    {
        return Ok(Ending::Inapplicable(
            "adapter supplies no controlled delivery clock".into(),
        ));
    }
    let mut session = case
        .open(
            "deadline-retry",
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("opening").0]),
                    ScriptedTurn::new(vec![case.mint.text("expiry-boundary").0]),
                ],
            },
        )
        .await?;
    send_and_wait(case, &mut session, None, "opening", &Map::new()).await?;
    let now = case
        .subject
        .delivery_clock_now_ms()
        .ok_or("adapter stopped supplying its delivery clock")?;
    let marker = case.mint.next("deadline-payload");
    let id = case.mint.next("send-id");
    let order = DeliveryOrder::new(vec![mode]).with_expires_at_ms(now + 1000);
    let original = params(case, &session, None, &marker, &id, &order.to_send_members());
    let receipt = expect_ok(
        "original send",
        call(&session.route, ops::SESSION_SEND, original.clone()).await?,
    )?;
    let first: SendReply = decode("original send", receipt.clone())?;
    if first.state != "pending" {
        return Err("fixture's future-deadline send did not stay pending".into());
    }
    let mut retry = original;
    retry["expires_at_ms"] = json!(now + 3000);
    let reply = expect_ok(
        "changed-deadline retry",
        call(&session.route, ops::SESSION_SEND, retry.clone()).await?,
    )?;
    if reply != receipt {
        return Err("changed deadline did not get the existing receipt".into());
    }
    case.subject
        .advance_retention_clock(&session.name, 1500)
        .await
        .map_err(|e| e.to_string())?;
    send_and_wait(case, &mut session, None, "expiry boundary", &Map::new()).await?;
    let reply: SendReply = decode(
        "expired retry",
        expect_ok(
            "expired retry",
            call(&session.route, ops::SESSION_SEND, retry).await?,
        )?,
    )?;
    if reply.state != "expired" {
        return Err(format!(
            "retry extended the original deadline: {}",
            reply.state
        ));
    }
    if read_all(&session.route)
        .await?
        .1
        .iter()
        .any(|entry| entry.message.to_string().contains(&marker))
    {
        return Err("expired send was delivered".into());
    }
    Ok(Ending::Passed)
}
