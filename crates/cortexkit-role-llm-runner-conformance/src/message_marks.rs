//! Check positive display marks over the same routes readers use.

use crate::{
    cases::{Case, Ending},
    drive::{
        call, decode, expect_error, expect_field, expect_ok, head, read, read_all, wait_run_end,
    },
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    delivery_order::PlanSender, errors, ops, read::ReadRequest, send::SendReply,
};
use serde_json::{json, Value};

fn same_marks(surface: &Value, expected: &Value, label: &str) -> Result<(), String> {
    for key in ["speaks_for", "notice"] {
        if surface.get(key) != expected.get(key) {
            return Err(format!(
                "{label} has incorrect {key}: {:?}, expected {:?}",
                surface.get(key),
                expected.get(key)
            ));
        }
    }
    Ok(())
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    if name == "message_marks_retry_changed" {
        return retry_changed(case).await;
    }
    let owner_only = name == "message_marks_speaks_for_owner_only";
    let stamps = if owner_only {
        let Some((carrier, owner)) = case.subject.scoped_session_start_stamps() else {
            return Ok(Ending::Inapplicable(
                "adapter cannot open a non-owner carrier route under a plain scope".into(),
            ));
        };
        if carrier.principal == owner.principal
            || carrier.scope != owner.scope
            || !owner
                .scope
                .as_ref()
                .is_some_and(|scope| scope.owner == owner.principal)
        {
            return Err(
                "plain-scope fixture must separate carrier and owner under the same scope".into(),
            );
        }
        Some((carrier, owner))
    } else {
        None
    };
    let mut session = case
        .open(
            "message-marks",
            Script {
                turns: (0..4)
                    .map(|_| ScriptedTurn::new(vec![case.mint.text("answer").0]))
                    .collect(),
            },
        )
        .await?;
    if let Some((_, owner)) = &stamps {
        session.route = case
            .subject
            .session_route(case.handle.inner(), &session.name, owner)
            .await
            .map_err(|e| e.to_string())?;
    }

    if name == "message_marks_served" {
        for marks in [
            json!({"notice":true}),
            json!({"speaks_for":{"agent_id":"agent-a"}}),
            json!({}),
        ] {
            let before = read_all(&session.route).await?.1;
            let prompt = case.mint.next("marked-prompt");
            let send_id = case.mint.next("marked-send");
            let mut params = case.params(&session, &prompt, &send_id, None);
            params
                .as_object_mut()
                .unwrap()
                .extend(marks.as_object().unwrap().clone());
            let reply: SendReply = decode(
                name,
                expect_ok(name, call(&session.route, ops::SESSION_SEND, params).await?)?,
            )?;
            wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
            session.first.get_or_insert((send_id, prompt));
            let (pages, defaults) = read_all(&session.route).await?;
            let version = pages
                .first()
                .ok_or("default read returned no page")?
                .history_version;
            let message = defaults
                .iter()
                .find(|m| {
                    m.role.as_deref() == Some("user") && !before.iter().any(|old| old.mid == m.mid)
                })
                .ok_or("send stored no user entry")?;
            same_marks(
                &serde_json::to_value(message).unwrap(),
                &json!({}),
                "default read",
            )?;
            let facts = read(
                &session.route,
                &ReadRequest::range(message.ordinal)
                    .with_limit(1)
                    .with_include_display_facts(true),
            )
            .await?;
            if facts.history_version != version {
                return Err("display facts changed history_version".into());
            }
            let fact = facts
                .messages
                .first()
                .filter(|m| m.mid == message.mid)
                .ok_or("opted-in read lost sent entry")?;
            same_marks(
                &serde_json::to_value(fact).unwrap(),
                &marks,
                "opted-in read",
            )?;
            if fact.message != message.message
                || fact.digest != message.digest
                || fact.part_digests != message.part_digests
            {
                return Err("marks changed stored body or checksum facts".into());
            }
            for display in [false, true] {
                let mut request = json!({"from_mid":message.mid, "limit":1});
                if display {
                    request["form"] = json!("display");
                }
                let page = expect_ok(
                    name,
                    call(&session.route, ops::SESSION_HISTORY, request).await?,
                )?;
                let entry = page["messages"]
                    .as_array()
                    .and_then(|m| m.first())
                    .filter(|m| m["mid"] == message.mid)
                    .ok_or("history lost sent entry")?;
                same_marks(
                    if display { &entry["row"] } else { entry },
                    &marks,
                    if display {
                        "display row"
                    } else {
                        "raw history"
                    },
                )?;
                if page["history_version"] != json!(version) {
                    return Err("history facts changed history_version".into());
                }
                if !display && entry["message"] != message.message {
                    return Err("raw history changed stored body".into());
                }
            }
        }
        return Ok(Ending::Passed);
    }

    let carrier_route = if let Some((carrier, _)) = &stamps {
        let send_id = case.mint.next("opening-send");
        let mut params = case.params(&session, "opening", &send_id, None);
        if case.declared.contains(&Capability::DeliveryOrder) {
            let members = serde_json::Map::from_iter([(
                "senders".into(),
                json!([
                    PlanSender::new(crate::send_helpers::stamp_module(name, carrier)?)
                        .with_idle_start()
                ]),
            )]);
            params.as_object_mut().unwrap().extend(
                case.subject
                    .send_fields_with_plan(&session.name, true, &members)
                    .ok_or("delivery_order needs plan fields for the carrier")?,
            );
        }
        let reply: SendReply = decode(
            name,
            expect_ok(name, call(&session.route, ops::SESSION_SEND, params).await?)?,
        )?;
        wait_run_end(case.subject, &session.route, case.declared, None, &reply).await?;
        session.first = Some((send_id, "opening".into()));
        Some(
            case.subject
                .session_route(case.handle.inner(), &session.name, carrier)
                .await
                .map_err(|e| e.to_string())?,
        )
    } else {
        None
    };
    let before = read_all(&session.route).await?;
    let before_head = head(&session.route).await?;
    let before_baseline = expect_ok(
        name,
        call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
    )?;
    let mut params = case.params(
        &session,
        "carried words",
        &case.mint.next("refused-send"),
        None,
    );
    params["speaks_for"] = json!({"agent_id":"agent-a"});
    if !owner_only {
        params["notice"] = json!(true);
    }
    let route = carrier_route.as_ref().unwrap_or(&session.route);
    if owner_only && case.declared.contains(&Capability::DeliveryOrder) {
        params["delivery"] = json!(["idle_start"]);
    }
    let error = expect_error(name, call(route, ops::SESSION_SEND, params).await?)?;
    expect_field(
        name,
        &error,
        errors::INVALID_PARAMS,
        if owner_only { "speaks_for" } else { "notice" },
    )?;
    if read_all(&session.route).await? != before
        || head(&session.route).await? != before_head
        || expect_ok(
            name,
            call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
        )? != before_baseline
    {
        return Err("refused marked send wrote history, head or baseline".into());
    }
    Ok(Ending::Passed)
}

async fn retry_changed<S: LlmRunnerSubject>(case: &Case<'_, S>) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let mut probes = vec![
        (json!({"notice": true}), json!({}), Some("notice")),
        (
            json!({"speaks_for": {"agent_id": "agent-a"}}),
            json!({"speaks_for": {"agent_id": "agent-b"}}),
            Some("speaks_for"),
        ),
        (json!({"notice": false}), json!({}), None),
    ];
    if case.declared.contains(&Capability::NoticeKeep) {
        probes.extend([
            (
                json!({"notice": true, "keep": true}),
                json!({"notice": true}),
                Some("keep"),
            ),
            (json!({"keep": false}), json!({}), None),
        ]);
    }
    let mut session = case
        .open(
            "marks-retry",
            Script {
                turns: probes
                    .iter()
                    .map(|_| ScriptedTurn::new(vec![case.mint.text("answer").0]))
                    .collect(),
            },
        )
        .await?;
    for (marks, changed, field) in probes {
        let send_id = case.mint.next("marks-retry-send");
        let prompt = case.mint.next("marks-retry-prompt");
        let mut params = case.params(&session, &prompt, &send_id, None);
        params
            .as_object_mut()
            .unwrap()
            .extend(marks.as_object().unwrap().clone());
        let accepted: SendReply = decode(
            "marked send",
            expect_ok(
                "marked send",
                call(&session.route, ops::SESSION_SEND, params.clone()).await?,
            )?,
        )?;
        wait_run_end(case.subject, &session.route, case.declared, None, &accepted).await?;
        session.first.get_or_insert((send_id, prompt));
        // Settle the run before comparing full answers: run state may advance
        // between acceptance and a retry, but retrying never creates a new send.
        let original = expect_ok(
            "identical marks",
            call(&session.route, ops::SESSION_SEND, params.clone()).await?,
        )?;
        let before = read_all(&session.route).await?;
        let before_head = head(&session.route).await?;
        let before_baseline = expect_ok(
            "baseline",
            call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
        )?;
        let replay = expect_ok(
            "identical marks",
            call(&session.route, ops::SESSION_SEND, params.clone()).await?,
        )?;
        let replayed: SendReply = decode("identical marks", replay.clone())?;
        if replay != original
            || replayed.run_id != accepted.run_id
            || replayed.submission_id != accepted.submission_id
        {
            return Err("identical marks did not replay the accepted send's answer".into());
        }
        let mut resent = params;
        for key in ["speaks_for", "notice", "keep"] {
            resent.as_object_mut().unwrap().remove(key);
        }
        resent
            .as_object_mut()
            .unwrap()
            .extend(changed.as_object().unwrap().clone());
        let response = call(&session.route, ops::SESSION_SEND, resent).await?;
        if let Some(field) = field {
            let label = format!("changed {field} marks");
            let error = expect_error(&label, response)?;
            expect_field(&label, &error, errors::SEND_ID_REUSE, field)?;
        } else if expect_ok("false equals absent", response)? != original {
            return Err("false/absent marks did not replay the accepted answer".into());
        }
        if read_all(&session.route).await? != before
            || head(&session.route).await? != before_head
            || expect_ok(
                "baseline",
                call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
            )? != before_baseline
        {
            return Err("marked retry wrote history, head or baseline".into());
        }
    }
    Ok(Ending::Passed)
}
