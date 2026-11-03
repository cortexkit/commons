//! Snapshot identity, full-path reads and run-fixed routing decisions.

use crate::{
    cases::{Case, Ending},
    drive::{
        call, expect_code, expect_error, expect_field, expect_ok, head, read, read_all,
        read_refusal,
    },
    route::RunnerRoute,
    send_helpers::send,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_llm_runner::{errors, history::HistoryPage, ops, read::ReadRequest};
use serde_json::json;

pub async fn run<S: LlmRunnerSubject>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    if name == "compaction_item_preset_required" {
        let session = case.open(name, Script::default()).await?;
        let mut params = case.params(
            &session,
            "not written",
            &case.mint.next("missing-preset"),
            None,
        );
        params["plan"]["compaction_item"] = json!({"provider":"example.compactor","params":{}});
        let before = head(&session.route).await?;
        let error = expect_error(
            "compaction plan without preset",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?;
        expect_field(
            "compaction plan without preset",
            &error,
            errors::INVALID_PARAMS,
            "plan.compaction_item.preset",
        )?;
        if head(&session.route).await? != before {
            return Err("missing-preset refusal wrote session state".into());
        }
        return Ok(Ending::Passed);
    }
    if name == "send_status_both_names" {
        let session = case.open(name, Script::default()).await?;
        let id = case.mint.next("unknown-send");
        for method in [ops::SESSION_SEND_STATUS, ops::SEND_STATUS] {
            let error = expect_error(
                method,
                call(&session.route, method, json!({"send_id":id})).await?,
            )?;
            expect_code(method, &error, errors::UNKNOWN_SEND)?;
        }
        return Ok(Ending::Passed);
    }
    if name == "route_decision_id_fixed_per_run" {
        let mut session = case
            .open(
                name,
                Script {
                    turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text("answer".into())])],
                },
            )
            .await?;
        let decision_id = case
            .subject
            .route_decision_id_fixture(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        case.subject
            .hold_next_model_step(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        send(
            case,
            &mut session,
            None,
            "start",
            &json!({"route_decision_id":decision_id})
                .as_object()
                .unwrap()
                .clone(),
        )
        .await?;
        case.subject
            .await_model_step(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        let mut joining = case.params(&session, "join", &case.mint.next("join-send"), None);
        joining["route_decision_id"] = json!(format!("{decision_id}-different"));
        joining["delivery"] = json!("steer");
        let reply = call(&session.route, ops::SESSION_SEND, joining).await;
        case.subject
            .release_model_step(&session.name)
            .await
            .map_err(|e| e.to_string())?;
        let error = expect_error("joining send", reply?)?;
        expect_field(
            "joining send",
            &error,
            errors::INVALID_PARAMS,
            "route_decision_id",
        )?;
        return Ok(Ending::Passed);
    }
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text("answer".into())])],
            },
        )
        .await?;
    case.run_prompt(&mut session, "prompt").await?;
    let snapshot = head(&session.route).await?;
    let one = read(&session.route, &ReadRequest::range(1).with_limit(1)).await?;
    match name {
        "read_from_ordinal_zero" => {
            let zero = read(&session.route, &ReadRequest::range(0).with_limit(1)).await?;
            if zero.messages != one.messages
                || zero.next_from_ordinal != one.next_from_ordinal
                || (zero.history_version, &zero.incarnation)
                    != (one.history_version, &one.incarnation)
                || one.messages.first().map(|entry| entry.ordinal) != Some(1)
            {
                return Err(
                    "from_ordinal 0 and 1 must both start at the first entry, numbered 1".into(),
                );
            }
            if case.declared.contains(&Capability::HostHistory) {
                let read_history = |from| {
                    call(
                        &session.route,
                        ops::SESSION_HISTORY,
                        json!({"from_ordinal":from,"limit":1}),
                    )
                };
                let zero: HistoryPage = crate::drive::decode(
                    "history from zero",
                    expect_ok("history from zero", read_history(0).await?)?,
                )?;
                let one: HistoryPage = crate::drive::decode(
                    "history from one",
                    expect_ok("history from one", read_history(1).await?)?,
                )?;
                if zero.messages != one.messages
                    || (zero.history_version, zero.incarnation)
                        != (one.history_version, one.incarnation)
                {
                    return Err(
                        "history from_ordinal 0 and 1 must reach the same first entry".into(),
                    );
                }
            }
        }
        "stale_read_cursor_history_changed" => {
            let first = one
                .messages
                .first()
                .ok_or("written session has no first entry")?;
            let stale = snapshot
                .history_version
                .checked_add(1)
                .unwrap_or_else(|| snapshot.history_version - 1);
            let error = read_refusal(
                &session.route,
                "stale after_mid",
                &ReadRequest::after(&first.mid, stale),
            )
            .await?;
            expect_code("stale after_mid", &error, errors::HISTORY_CHANGED)?;
        }
        "history_version_on_read_pages" | "incarnation_on_every_page" => {
            let mut request = ReadRequest::range(0).with_limit(1);
            let mut pages = 0;
            loop {
                pages += 1;
                if pages > crate::drive::MAX_POLLS {
                    return Err("read page walk never reached its explicit end".into());
                }
                let page = read(&session.route, &request).await?;
                if page.history_version != snapshot.history_version
                    || page.incarnation != snapshot.incarnation
                {
                    return Err(
                        "read page does not carry the head's (history_version, incarnation) pair"
                            .into(),
                    );
                }
                match page.next_from_ordinal {
                    Some(next) => request = ReadRequest::range(next).with_limit(1),
                    None => break,
                }
            }
            if case.declared.contains(&Capability::HostHistory) {
                for display in [false, true] {
                    if display && !case.declared.contains(&Capability::HistoryDisplay) {
                        continue;
                    }
                    let mut params = json!({"limit":1});
                    if display {
                        params["form"] = json!("display");
                    }
                    let mut ended = false;
                    for _ in 0..crate::drive::MAX_POLLS {
                        let value = expect_ok(
                            "session.history",
                            call(&session.route, ops::SESSION_HISTORY, params.clone()).await?,
                        )?;
                        let (version, incarnation, older) = if display {
                            let page: cortexkit_role_llm_runner::display::DisplayPage =
                                crate::drive::decode("display history", value)?;
                            (page.history_version, page.incarnation, page.older)
                        } else {
                            let page: HistoryPage = crate::drive::decode("raw history", value)?;
                            (page.history_version, page.incarnation, page.older)
                        };
                        if version != snapshot.history_version
                            || incarnation != snapshot.incarnation
                        {
                            return Err("history page does not carry the head's cache pair".into());
                        }
                        match older {
                            Some(cursor) => params["cursor"] = json!(cursor),
                            None => {
                                ended = true;
                                break;
                            }
                        }
                    }
                    if !ended {
                        return Err("history page walk never reached its explicit end".into());
                    }
                }
            }
        }
        "read_below_host_marker" => {
            let (_, before) = read_all(&session.route).await?;
            if let Err(error) = case
                .subject
                .compact_in_host(case.handle.inner(), &session.name)
                .await
            {
                return Ok(Ending::Inapplicable(error.to_string()));
            }
            let after_head = head(&session.route).await?;
            if after_head.cut.is_none() {
                return Ok(Ending::Inapplicable(
                    "the native hook produced no model marker".into(),
                ));
            }
            if after_head.history_version != snapshot.history_version
                || after_head.incarnation != snapshot.incarnation
            {
                return Err("a native marker changed the history cache pair".into());
            }
            let (_, after) = read_all(&session.route).await?;
            if before != after {
                return Err("a host marker hid or renumbered entries from session.read".into());
            }
        }
        other => return Err(format!("unknown read contract case {other}")),
    }
    Ok(Ending::Passed)
}
