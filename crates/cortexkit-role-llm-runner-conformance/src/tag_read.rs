use cortexkit_role_llm_runner::{
    errors, ops,
    read::{ReadRequest, TagReadAnswer, TagReadRequest},
    tool_calls::ToolCallsPage,
};
use serde_json::json;

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_error, expect_field, expect_ok, head, read, read_all},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let scripted_call = case.mint.call("tag-citation");
    let mut session = case
        .open(
            name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![case.mint.text("ready").0]),
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(scripted_call.clone())]),
                    ScriptedTurn::new(vec![case.mint.text("done").0]),
                    ScriptedTurn::new(vec![case.mint.text("after-undo").0]),
                ],
            },
        )
        .await?;
    if name == "tag_read_undeclared" {
        let before = head(&session.route).await?;
        let error = expect_error(
            name,
            call(
                &session.route,
                ops::SESSION_READ,
                json!({"tag":1,"call_key":"no-call"}),
            )
            .await?,
        )?;
        expect_field(name, &error, errors::INVALID_PARAMS, "tag")?;
        if head(&session.route).await? != before {
            return Err("undeclared tag read wrote session state".into());
        }
        return Ok(Ending::Passed);
    }

    case.run_prompt(&mut session, "establish the session")
        .await?;
    let marker = case.mint.next("person-words");
    let resolution = matches!(
        name,
        "tag_read_person" | "tag_read_before_cut" | "tag_read_before_undo"
    );
    let target = if resolution {
        let Some((mid, part)) = case
            .subject
            .seed_tag_read_person(case.handle.inner(), &session.name, &marker)
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(Ending::Inapplicable(
                "adapter cannot seed a person's tagged entry".into(),
            ));
        };
        let page = read(
            &session.route,
            &ReadRequest::range(1).with_include_display_facts(true),
        )
        .await?;
        let entry = page
            .messages
            .into_iter()
            .find(|entry| entry.mid == mid)
            .ok_or("seeded person entry is absent from ordinary read")?;
        let parts = case.subject.read_parts(&entry.message);
        let key = parts
            .get(part as usize)
            .ok_or("seeded part index is outside its entry")?
            .0
            .clone();
        if part == 0
            || entry.role.as_deref() != Some("user")
            || entry.notice
            || entry.speaks_for.is_some()
            || entry.client_message_id.as_deref() != Some(marker.as_str())
            || !entry.message.to_string().contains(&marker)
        {
            return Err("person fixture must seed an unmarked user entry with app id and a nonzero marked part".into());
        }
        Some((entry, part, key))
    } else {
        None
    };
    let (_, end) = case
        .run_prompt(&mut session, "make the citing call")
        .await?;
    let calls: ToolCallsPage = decode(
        name,
        expect_ok(
            name,
            call(
                &session.route,
                ops::RUN_TOOL_CALLS,
                json!({"run_id":end.run_id}),
            )
            .await?,
        )?,
    )?;
    let call_key = calls
        .calls
        .iter()
        .find(|call| call.tool_call_id == scripted_call.tool_call_id)
        .and_then(|call| call.call_key.clone())
        .ok_or("scripted call has no call_key")?;

    if let Some((entry, part, key)) = target {
        let Some(tag) = case
            .subject
            .shown_part_number(&session.name, &entry.mid, &key)
        else {
            return Ok(Ending::Inapplicable(
                "adapter cannot observe model-visible part numbers".into(),
            ));
        };
        if name == "tag_read_before_cut" {
            if let Err(error) = case
                .subject
                .compact_in_host(case.handle.inner(), &session.name)
                .await
            {
                return Ok(Ending::Inapplicable(format!(
                    "adapter cannot produce a cut: {error}"
                )));
            }
            let cut = head(&session.route).await?;
            if cut.cut.is_none() || !cut.first_ordinal.is_some_and(|first| first > entry.ordinal) {
                return Ok(Ending::Inapplicable(
                    "compaction fixture did not cut the target entry".into(),
                ));
            }
        }
        if name == "tag_read_before_undo" {
            let before = read_all(&session.route).await?.1;
            case.run_prompt(&mut session, "make a later turn to undo")
                .await?;
            let later = read_all(&session.route)
                .await?
                .1
                .into_iter()
                .find(|message| {
                    message.role.as_deref() == Some("user")
                        && !before.iter().any(|old| old.mid == message.mid)
                })
                .ok_or("later turn stored no user entry")?;
            if let Err(error) = case
                .subject
                .revert_session(case.handle.inner(), &session.name, &later.mid)
                .await
            {
                return Ok(Ending::Inapplicable(format!(
                    "adapter cannot undo: {error}"
                )));
            }
        }
        let request = TagReadRequest::new(tag, call_key.clone());
        let answer: TagReadAnswer = decode(
            name,
            expect_ok(
                name,
                call(
                    &session.route,
                    ops::SESSION_READ,
                    serde_json::to_value(request).unwrap(),
                )
                .await?,
            )?,
        )?;
        if answer.part != part
            || answer.entry.mid != entry.mid
            || answer.entry.message != entry.message
            || answer.entry.sender != entry.sender
            || answer.entry.notice != entry.notice
            || answer.entry.keep != entry.keep
            || answer.entry.speaks_for != entry.speaks_for
            || answer.entry.client_message_id != entry.client_message_id
        {
            return Err(format!(
                "call-fixed tag resolved the wrong entry, part or display facts: {answer:?}"
            ));
        }
        if name == "tag_read_before_undo" {
            if let Err(error) = case
                .subject
                .revert_session(case.handle.inner(), &session.name, &entry.mid)
                .await
            {
                return Ok(Ending::Inapplicable(format!(
                    "adapter cannot undo: {error}"
                )));
            }
            let before = (head(&session.route).await?, read_all(&session.route).await?);
            let error = expect_error(
                name,
                call(
                    &session.route,
                    ops::SESSION_READ,
                    json!({"tag":tag,"call_key":call_key}),
                )
                .await?,
            )?;
            if error.code != errors::UNKNOWN_CALL {
                return Err(format!(
                    "removed call refused {}, not unknown_call",
                    error.code
                ));
            }
            if (head(&session.route).await?, read_all(&session.route).await?) != before {
                return Err("refused tag read wrote session state".into());
            }
        }
        return Ok(Ending::Passed);
    }

    let (route, params, expected) = match name {
        "tag_read_unknown_call" => {
            let mut foreign = case
                .open(
                    "foreign-tag-call",
                    Script {
                        turns: vec![
                            ScriptedTurn::new(vec![ScriptedPart::ToolCall(
                                case.mint.call("foreign-citation"),
                            )]),
                            ScriptedTurn::new(vec![case.mint.text("foreign-done").0]),
                        ],
                    },
                )
                .await?;
            let (_, end) = case.run_prompt(&mut foreign, "make a foreign call").await?;
            let calls: ToolCallsPage = decode(
                name,
                expect_ok(
                    name,
                    call(
                        &foreign.route,
                        ops::RUN_TOOL_CALLS,
                        json!({"run_id":end.run_id}),
                    )
                    .await?,
                )?,
            )?;
            let foreign_key = calls
                .calls
                .first()
                .and_then(|call| call.call_key.clone())
                .ok_or("foreign call has no call_key")?;
            (
                &session.route,
                json!({"tag":0,"call_key":foreign_key}),
                errors::UNKNOWN_CALL,
            )
        }
        "tag_read_unknown_tag" => (
            &session.route,
            json!({"tag":0,"call_key":call_key}),
            errors::UNKNOWN_TAG,
        ),
        "tag_read_non_owner" => {
            let route = case
                .subject
                .session_route(
                    case.handle.inner(),
                    &session.name,
                    &case.subject.stranger_stamp(),
                )
                .await
                .map_err(|e| e.to_string())?;
            let error = expect_error(
                name,
                call(
                    &route,
                    ops::SESSION_READ,
                    json!({"tag":0,"call_key":call_key}),
                )
                .await?,
            )?;
            if error.code != errors::SCOPE_OWNER_MISMATCH {
                return Err(format!(
                    "non-owner tag read refused {}, not scope_owner_mismatch",
                    error.code
                ));
            }
            return Ok(Ending::Passed);
        }
        _ => return Err(format!("unknown tag read case {name}")),
    };
    let before = (head(&session.route).await?, read_all(&session.route).await?);
    let error = expect_error(name, call(route, ops::SESSION_READ, params).await?)?;
    if error.code != expected {
        return Err(format!("{name} refused {}, not {expected}", error.code));
    }
    if (head(&session.route).await?, read_all(&session.route).await?) != before {
        return Err("refused tag read wrote session state".into());
    }
    if name == "tag_read_unknown_tag" {
        let Some((mid, part)) = case
            .subject
            .seed_tag_read_person(
                case.handle.inner(),
                &session.name,
                &case.mint.next("later-tag"),
            )
            .await
            .map_err(|e| e.to_string())?
        else {
            return Ok(Ending::Inapplicable(
                "adapter cannot seed a tag assigned after a call".into(),
            ));
        };
        case.run_prompt(&mut session, "show a tag the old call never saw")
            .await?;
        let page = read(
            &session.route,
            &ReadRequest::range(1).with_include_display_facts(true),
        )
        .await?;
        let entry = page
            .messages
            .iter()
            .find(|entry| entry.mid == mid)
            .ok_or("later entry is absent")?;
        let parts = case.subject.read_parts(&entry.message);
        let key = &parts.get(part as usize).ok_or("later part is absent")?.0;
        let Some(tag) = case.subject.shown_part_number(&session.name, &mid, key) else {
            return Ok(Ending::Inapplicable(
                "adapter cannot observe the later tag".into(),
            ));
        };
        let error = expect_error(
            name,
            call(
                &session.route,
                ops::SESSION_READ,
                json!({"tag":tag,"call_key":call_key}),
            )
            .await?,
        )?;
        if error.code != errors::UNKNOWN_TAG {
            return Err(format!("later tag refused {}, not unknown_tag", error.code));
        }
    }
    Ok(Ending::Passed)
}
