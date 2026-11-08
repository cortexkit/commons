use cortexkit_role_llm_runner::{
    delivery_order::PlanSender, errors, ops, read::ReadRequest, send::SendReply,
};
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::{
        call, decode, expect_error, expect_field, expect_ok, head, read, read_all, wait_run_end,
    },
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedTurn},
};

fn marks(surface: &Value, expected: &Value, label: &str) -> Result<(), String> {
    for key in ["notice", "keep"] {
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
    let owner_only = name == "notice_keep_owner_only";
    let stamps = if owner_only {
        let Some((carrier, owner)) = case.subject.scoped_session_start_stamps() else {
            return Ok(Ending::Inapplicable(
                "adapter cannot open a non-owner carrier under a plain scope".into(),
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
            "notice-keep",
            Script {
                turns: (0..3)
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

    if matches!(
        name,
        "notice_keep_owner_served" | "notice_keep_false_absent"
    ) {
        let expected = if name == "notice_keep_owner_served" {
            json!({"notice":true,"keep":true})
        } else {
            json!({})
        };
        // Compare explicit false with an ordinary unmarked send as well as
        // checking omission. Absence must not acquire a keep mark either.
        let sends = if name == "notice_keep_false_absent" {
            vec![json!({"keep":false}), json!({})]
        } else {
            vec![expected.clone()]
        };
        for sent in sends {
            let before = read_all(&session.route).await?.1;
            let prompt = case.mint.next("kept-words");
            let send_id = case.mint.next("kept-send");
            let mut params = case.params(&session, &prompt, &send_id, None);
            params
                .as_object_mut()
                .unwrap()
                .extend(sent.as_object().unwrap().clone());
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
            marks(
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
            let fact = facts
                .messages
                .first()
                .filter(|m| m.mid == message.mid)
                .ok_or("opted-in read lost sent entry")?;
            marks(
                &serde_json::to_value(fact).unwrap(),
                &expected,
                "opted-in read",
            )?;
            if facts.history_version != version
                || fact.message != message.message
                || fact.digest != message.digest
                || fact.part_digests != message.part_digests
            {
                return Err("keep changed stored body, checksum facts or history_version".into());
            }
            for display in [false, true] {
                let mut request = json!({"from_mid":message.mid,"limit":1});
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
                marks(
                    if display { &entry["row"] } else { entry },
                    &expected,
                    if display {
                        "display row"
                    } else {
                        "raw history"
                    },
                )?;
                if page["history_version"] != json!(version)
                    || (!display && entry["message"] != message.message)
                {
                    return Err("keep changed history body or history_version".into());
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
    let values = if name == "notice_keep_undeclared" {
        vec![false, true]
    } else {
        vec![true]
    };
    for keep in values {
        let before = read_all(&session.route).await?;
        let before_head = head(&session.route).await?;
        let before_baseline = expect_ok(
            name,
            call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
        )?;
        let mut params = case.params(
            &session,
            "person's words",
            &case.mint.next("refused-keep"),
            None,
        );
        params["keep"] = json!(keep);
        if !matches!(
            name,
            "notice_keep_requires_notice" | "notice_keep_undeclared"
        ) {
            params["notice"] = json!(true);
        }
        let field = if name == "notice_keep_tool_result_refused" {
            params["delivery"] = json!(["steer", "tool_result"]);
            "delivery"
        } else {
            "keep"
        };
        if owner_only && case.declared.contains(&Capability::DeliveryOrder) {
            params["delivery"] = json!(["idle_start"]);
        }
        let route = carrier_route.as_ref().unwrap_or(&session.route);
        let error = expect_error(name, call(route, ops::SESSION_SEND, params).await?)?;
        expect_field(name, &error, errors::INVALID_PARAMS, field)?;
        if read_all(&session.route).await? != before
            || head(&session.route).await? != before_head
            || expect_ok(
                name,
                call(&session.route, ops::SESSION_BASELINE, json!({})).await?,
            )? != before_baseline
        {
            return Err("refused kept send wrote history, head or baseline".into());
        }
    }
    Ok(Ending::Passed)
}
