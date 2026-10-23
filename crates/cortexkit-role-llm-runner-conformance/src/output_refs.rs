//! Immutable output-reference records, observed through the runner's routes.

use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok, read_all},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use cortexkit_role_llm_runner::{
    capabilities as groups,
    content_digest::sha256_hex,
    describe::RoleDescribe,
    ops,
    output_refs::{Descriptor, Page, ReadReply, Record, MAX_READ_BYTES, MAX_SNAPSHOT_BYTES},
};
use serde_json::{json, Map, Value};

type Check<T> = Result<T, String>;

async fn records<R: RunnerRoute>(route: &R, descriptor: &Descriptor) -> Check<Vec<Record>> {
    let mut after = None;
    let mut all = Vec::new();
    loop {
        let reply = expect_ok(
            "session.output_refs",
            call(
                route,
                ops::SESSION_OUTPUT_REFS,
                json!({"set_id":descriptor.set_id,"after":after}),
            )
            .await?,
        )?;
        if serde_json::to_vec(&reply).unwrap().len() > 16_384 {
            return Err("output_refs page exceeds 16384 bytes".into());
        }
        let page: Page = decode("output reference page", reply)?;
        if page.records.len() > 32 {
            return Err("output_refs page exceeds 32 records".into());
        }
        if page.records.is_empty() && page.next.is_some() {
            return Err("empty output_refs page does not advance".into());
        }
        all.extend(page.records);
        if all.len() > descriptor.count as usize {
            return Err("output_refs page repeated records".into());
        }
        match page.next {
            None => break,
            Some(next) if after.as_ref() == Some(&next) => {
                return Err("output_refs cursor did not advance".into())
            }
            Some(next) => after = Some(next),
        }
    }
    Ok(all)
}

async fn snapshot<R: RunnerRoute>(route: &R, record: &Record) -> Check<Vec<u8>> {
    let reply: ReadReply = decode(
        "output snapshot",
        expect_ok(
            "output snapshot",
            call(
                route,
                ops::SESSION_OUTPUT_REF_READ,
                json!({"ref_id":record.ref_id,"offset":0,"length":MAX_READ_BYTES}),
            )
            .await?,
        )?,
    )?;
    STANDARD.decode(reply.data).map_err(|e| e.to_string())
}

async fn descriptors<R: RunnerRoute>(route: &R) -> Check<Vec<Descriptor>> {
    let page = expect_ok(
        "display history",
        call(
            route,
            ops::SESSION_HISTORY,
            json!({"form":"display","limit":1000,"max_bytes":4_000_000}),
        )
        .await?,
    )?;
    let mut out = Vec::new();
    for entry in page["messages"]
        .as_array()
        .ok_or("display history has no entries")?
    {
        let row = &entry["row"];
        if let Some(d) = row.get("output_refs") {
            out.push(decode("message output_refs", d.clone())?);
        }
        for part in row["parts"].as_array().into_iter().flatten() {
            if let Some(d) = part.get("output_refs") {
                out.push(decode("call output_refs", d.clone())?);
            }
        }
    }
    Ok(out)
}

fn tag_of(
    messages: &[cortexkit_role_llm_runner::read::ReadMessage],
    marker: &str,
) -> Check<String> {
    let message = messages
        .iter()
        .find(|m| m.message.to_string().contains(marker))
        .ok_or("fixture message missing")?;
    message
        .tags
        .iter()
        .find_map(|t| t.tag.as_ref())
        .map(|tag| tag.trim_matches('§').to_owned())
        .ok_or("fixture message has no rendered tag".into())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Check<Ending>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    let mut session = case.open(name, Script::default()).await?;
    let describe: RoleDescribe = decode(
        "role.describe",
        expect_ok(
            "role.describe",
            call(&session.route, ops::ROLE_DESCRIBE, json!({})).await?,
        )?,
    )?;
    if name == "output_refs_unsupported_plan" {
        if describe.declares(groups::OUTPUT_REFS) {
            return Ok(Ending::Inapplicable(
                "the runner declares output_refs".into(),
            ));
        }
        let observation = if describe.declares(groups::TRANSCRIPT_READS) {
            ops::SESSION_HEAD
        } else {
            ops::SESSION_BASELINE
        };
        for flag in [true, false] {
            let before = expect_ok(
                "state before refusal",
                call(&session.route, observation, json!({})).await?,
            )?;
            let mut fields = case.subject.send_fields(&session.name, true);
            let plan = fields.entry("plan").or_insert_with(|| json!({}));
            if plan
                .get("items")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
            {
                plan["items"][0]["output_refs"] = json!(flag);
            } else {
                plan["items"] = json!([{"output_refs":flag}]);
            }
            let plan = plan.clone();
            fields.insert("send_id".into(), json!(case.mint.next("send")));
            fields.insert("prompt".into(), json!("must not be stored"));
            let mut operations = vec![(ops::SESSION_SEND, Value::Object(fields))];
            if describe.declares(groups::SESSION_CHANGE) {
                operations.push((
                    ops::SESSION_REFRESH,
                    json!({"plan":plan,"policy":"immediately","generation":1}),
                ));
            }
            if describe.declares(groups::ADOPT) {
                operations.push((
                    ops::SESSION_ADOPT,
                    json!({"plan":plan,"send_id":case.mint.next("adopt")}),
                ));
            }
            for (op, params) in operations {
                expect_field(
                    op,
                    &expect_error(op, call(&session.route, op, params).await?)?,
                    "invalid_params",
                    "plan.items[].output_refs",
                )?;
                let after = expect_ok(
                    "state after refusal",
                    call(&session.route, observation, json!({})).await?,
                )?;
                if before != after {
                    return Err(format!("{op} wrote before refusing output_refs"));
                }
            }
        }
        return Ok(Ending::Passed);
    }
    if !describe.declares(groups::OUTPUT_REFS) {
        return Ok(Ending::Inapplicable(
            "the runner does not declare output_refs".into(),
        ));
    }
    if !describe.declares(groups::TAGS) {
        return Err("output_refs requires tags".into());
    }
    let fields = case
        .subject
        .output_refs_fields(&session.name)
        .map_err(|e| e.to_string())?;
    let marker = case.mint.next("stored-output");
    let user_marker = case.mint.next("user-text");
    let model_marker = case.mint.next("model-text");
    let mut tool = case.mint.call("refs-source");
    let oversized = name == "output_refs_snapshot_size";
    let budget = name == "output_refs_snapshot_budget";
    let content = if oversized {
        "x".repeat(MAX_SNAPSHOT_BYTES + 1)
    } else if budget {
        "x".repeat(MAX_SNAPSHOT_BYTES)
    } else {
        marker.clone()
    };
    tool.result = json!(content);
    case.subject
        .install_script(
            &session.name,
            Script {
                turns: vec![
                    ScriptedTurn::new(vec![ScriptedPart::ToolCall(tool)]),
                    ScriptedTurn::new(vec![ScriptedPart::Text(model_marker.clone())]),
                ],
            },
        )
        .await
        .map_err(|e| e.to_string())?;
    crate::tags::send_and_wait(case, &mut session, Some(&fields), &user_marker, &Map::new())
        .await?;
    let (_, messages) = read_all(&session.route).await?;
    let tag = tag_of(
        &messages,
        if oversized || budget {
            &content[..100]
        } else {
            &marker
        },
    )?;
    let user_tag = tag_of(&messages, &user_marker)?;
    let model_tag = tag_of(&messages, &model_marker)?;
    let mut held_tag = None;
    if name == "output_refs_held_placeholder" {
        held_tag = Some(
            case.subject
                .install_output_ref_held(case.handle.inner(), &session.name, "held placeholder")
                .await
                .map_err(|e| e.to_string())?,
        );
    }
    let text = match name {
        "output_refs_scanning" => format!("```\n$$tc({tag})\n```\n`$$tc({tag})` \\$$tc({tag})"),
        "output_refs_target_reasons" => {
            format!("$$tc({user_tag}) $$tc({model_tag}) $$tc(99999999999999999999999999999)")
        }
        "output_refs_held_placeholder" => format!("$$tc({})", held_tag.as_ref().unwrap()),
        "output_refs_reference_limit" => format!("$$tc({tag}) ").repeat(65),
        "output_refs_snapshot_budget" => format!("$$tc({tag}) ").repeat(5),
        _ => format!("$$tc({tag})"),
    };
    let mut parts = vec![ScriptedPart::Text(text)];
    if name == "output_refs_slots" {
        let mut call = case.mint.call("refs-slots");
        call.arguments = json!({"nonce":case.mint.next("arg"),"data":format!("extra $$tc({tag})"),"program":format!("export const x = $$tc({tag})\nexport const y = \"$$tc({tag})\""),"other":format!("$$tc({tag})")});
        parts.push(ScriptedPart::ToolCall(call));
    }
    if name == "output_refs_shared_samples" {
        parts.clear();
        for samples in [
            cortexkit_rich_ui_syntax::vectors::ORIGINAL_SAMPLES,
            cortexkit_rich_ui_syntax::vectors::EXTENDED_SAMPLES,
        ] {
            let samples: Value = serde_json::from_str(samples).unwrap();
            for sample in samples["cases"].as_array().unwrap() {
                if sample["mode"] == "reply" {
                    parts.push(ScriptedPart::Text(
                        sample["source"].as_str().unwrap().into(),
                    ));
                } else {
                    let mut call = case.mint.call("refs-sample");
                    let slot = if sample["mode"] == "token_slot" {
                        "data"
                    } else {
                        "program"
                    };
                    call.arguments = json!({"nonce":case.mint.next("arg"),slot:sample["source"]});
                    parts.push(ScriptedPart::ToolCall(call));
                }
            }
        }
    }
    // Preserve the first two consumed answers when replacing the script.
    let mut script = Script {
        turns: vec![
            ScriptedTurn::default(),
            ScriptedTurn::default(),
            ScriptedTurn::new(parts),
            ScriptedTurn::new(vec![ScriptedPart::Text("done".into())]),
        ],
    };
    if name != "output_refs_slots" && name != "output_refs_shared_samples" {
        script.turns.pop();
    }
    case.subject
        .install_script(&session.name, script)
        .await
        .map_err(|e| e.to_string())?;
    let before_send = messages.last().ok_or("no initial message")?.mid.clone();
    crate::tags::send_and_wait(case, &mut session, None, "write references", &Map::new()).await?;
    let sets = descriptors(&session.route).await?;
    let d = sets
        .iter()
        .find(|d| d.count > 0)
        .or_else(|| sets.last())
        .ok_or("display row has no output_refs descriptor")?
        .clone();
    let r = records(&session.route, &d).await?;
    match name {
        "output_refs_scanning" => {
            if !r.is_empty() {
                return Err("code or escaped reference copied output".into());
            }
        }
        "output_refs_target_reasons" => {
            let reasons: Vec<_> = r.iter().map(|r| r.reason.as_deref()).collect();
            if reasons
                != [
                    Some("not_a_tool_output"),
                    Some("not_a_tool_output"),
                    Some("unknown_tag"),
                ]
            {
                return Err(format!("wrong target reasons: {reasons:?}"));
            }
        }
        "output_refs_snapshot_size" => {
            let record = r.first().ok_or("large reference missing")?;
            if record.state != "too_large"
                || record.bytes != Some(content.len() as u64)
                || record.sha256.as_deref() != Some(sha256_hex(content.as_bytes()).as_str())
            {
                return Err("large snapshot lacks full size and digest".into());
            }
        }
        "output_refs_snapshot_budget" => {
            if r.len() != 5
                || r[..4].iter().any(|r| r.state != "resolved")
                || r[4].state != "too_large"
                || r[4].bytes != Some(content.len() as u64)
                || r[4].sha256.as_deref() != Some(sha256_hex(content.as_bytes()).as_str())
            {
                return Err("aggregate snapshot budget did not preserve full metadata".into());
            }
            expect_code(
                "over-budget content",
                &expect_error(
                    "over-budget content",
                    call(
                        &session.route,
                        ops::SESSION_OUTPUT_REF_READ,
                        json!({"ref_id":r[4].ref_id,"offset":0,"length":1}),
                    )
                    .await?,
                )?,
                "unknown_ref",
            )?;
        }
        "output_refs_descriptor_digest" => {
            if !d.check(&r) {
                return Err("descriptor digest does not match canonical records".into());
            }
        }
        "output_refs_slots" => {
            let call = sets
                .iter()
                .find(|call| call.call_key.is_some() && call.mid == d.mid)
                .ok_or("call set missing")?;
            let r = records(&session.route, call).await?;
            if r.len() != 2
                || r[0].reason.as_deref() != Some("not_a_reference")
                || r[1].state != "resolved"
            {
                return Err("slot-only or exact-token rule broken".into());
            }
        }
        "output_refs_shared_samples" => {
            let mut expected_reply = Vec::new();
            let mut expected_calls = Vec::new();
            for samples in [
                cortexkit_rich_ui_syntax::vectors::ORIGINAL_SAMPLES,
                cortexkit_rich_ui_syntax::vectors::EXTENDED_SAMPLES,
            ] {
                let samples: Value = serde_json::from_str(samples).unwrap();
                for sample in samples["cases"].as_array().unwrap() {
                    let tags: Vec<String> = sample["references"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|r| {
                            r["n"]
                                .as_str()
                                .map(str::to_owned)
                                .unwrap_or_else(|| r["n"].to_string())
                        })
                        .collect();
                    if sample["mode"] == "reply" {
                        expected_reply.extend(tags);
                    } else {
                        expected_calls.push(tags);
                    }
                }
            }
            let got: Vec<_> = r.iter().filter_map(|r| r.tag.clone()).collect();
            if got != expected_reply {
                return Err("reply scanner differs from published samples".into());
            }
            let calls: Vec<_> = sets
                .iter()
                .filter(|call| call.call_key.is_some() && call.mid == d.mid)
                .collect();
            if calls.len() != expected_calls.len() {
                return Err("sample call sets missing".into());
            }
            for (d, expected) in calls.into_iter().zip(expected_calls) {
                let got: Vec<_> = records(&session.route, d)
                    .await?
                    .iter()
                    .filter_map(|r| r.tag.clone())
                    .collect();
                if got != expected {
                    return Err("program/token scanner differs from published samples".into());
                }
            }
        }
        "output_refs_reference_limit" => {
            if r.len() != 65 || r[64].reason.as_deref() != Some("too_many_refs") {
                return Err("reference cap did not span the message".into());
            }
        }
        "output_refs_session_isolation" => {
            let other = case.open("foreign-output-refs", Script::default()).await?;
            for (op, params) in [
                (ops::SESSION_OUTPUT_REFS, json!({"set_id":d.set_id})),
                (
                    ops::SESSION_OUTPUT_REF_READ,
                    json!({"ref_id":r[0].ref_id,"offset":0,"length":0}),
                ),
            ] {
                expect_code(
                    "foreign output reference",
                    &expect_error(
                        "foreign output reference",
                        call(&other.route, op, params).await?,
                    )?,
                    "unknown_ref",
                )?;
            }
        }
        "output_refs_rewind" => {
            case.subject
                .revert_session(case.handle.inner(), &session.name, &before_send)
                .await
                .map_err(|e| e.to_string())?;
            expect_code(
                "removed set",
                &expect_error(
                    "removed set",
                    call(
                        &session.route,
                        ops::SESSION_OUTPUT_REFS,
                        json!({"set_id":d.set_id}),
                    )
                    .await?,
                )?,
                "unknown_ref",
            )?;
        }
        "output_refs_compaction"
        | "output_refs_writing_request"
        | "output_refs_held_placeholder" => {
            if name == "output_refs_compaction" {
                case.subject
                    .compact_in_host(case.handle.inner(), &session.name)
                    .await
                    .map_err(|e| e.to_string())?;
            } else if name == "output_refs_writing_request" {
                case.subject
                    .renumber_output_refs(case.handle.inner(), &session.name)
                    .await
                    .map_err(|e| e.to_string())?;
            } else {
                case.subject
                    .complete_output_ref_held(case.handle.inner(), &session.name, "late answer")
                    .await
                    .map_err(|e| e.to_string())?;
            }
            let after = records(&session.route, &d).await?;
            if after != r {
                return Err("stored records changed after the writing commit".into());
            }
            let expected = if name == "output_refs_held_placeholder" {
                b"held placeholder".as_slice()
            } else {
                marker.as_bytes()
            };
            if snapshot(&session.route, &r[0]).await? != expected {
                return Err("stored snapshot changed or contains rendered bytes".into());
            }
            if name == "output_refs_held_placeholder"
                && (r[0].state != "held" || r[0].call_key.is_none())
            {
                return Err("held placeholder lost state or call_key".into());
            }
        }
        "output_refs_read_limits" => {
            for (offset, length, field) in [
                (0, MAX_READ_BYTES + 1, "length"),
                (marker.len() as u64 + 1, 0, "offset"),
            ] {
                expect_field(
                    "read bounds",
                    &expect_error(
                        "read bounds",
                        call(
                            &session.route,
                            ops::SESSION_OUTPUT_REF_READ,
                            json!({"ref_id":r[0].ref_id,"offset":offset,"length":length}),
                        )
                        .await?,
                    )?,
                    "invalid_params",
                    field,
                )?;
            }
            for (offset, length) in [(0, 0), (marker.len() as u64, 1)] {
                let reply: ReadReply = decode(
                    "metadata read",
                    expect_ok(
                        "metadata read",
                        call(
                            &session.route,
                            ops::SESSION_OUTPUT_REF_READ,
                            json!({"ref_id":r[0].ref_id,"offset":offset,"length":length}),
                        )
                        .await?,
                    )?,
                )?;
                if !reply.data.is_empty() || reply.size != marker.len() as u64 {
                    return Err("metadata/EOF read differs".into());
                }
            }
        }
        _ => return Err(format!("unknown output_refs case {name}")),
    }
    // Descriptors are display metadata, not stored-body or default-read members.
    let raw = expect_ok(
        "default read",
        call(&session.route, ops::SESSION_READ, json!({})).await?,
    )?;
    if raw.to_string().contains("\"output_refs\"") {
        return Err("default read leaked output_refs metadata".into());
    }
    Ok(Ending::Passed)
}
