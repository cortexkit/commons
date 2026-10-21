//! Descriptor reads observed on the runner route, with actual media bytes
//! supplied by the adapter and digests computed from separate inline reads.
use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok, wait_run_end},
    route::RunnerRoute,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    content_digest::{content_digest, sha256_hex},
    errors, ops,
    read::{descriptors::PART_INLINE_MAX, MediaMode, ReadMessage, ReadPage, ReadRequest},
    send::SendReply,
};
use serde_json::{json, Value};

type Check = Result<(), String>;

const MIXED_FALLBACK_SIZES: &[usize] = &[
    128_000, 230_000, 160_000, 240_000, 180_000, 230_000, 180_000, 180_000, 180_000, 180_000,
    180_000, 180_000, 180_000, 180_000, 180_000, 180_000, 180_000, 180_000, 180_000,
];

fn mixed_fallback_texts() -> Vec<String> {
    MIXED_FALLBACK_SIZES
        .iter()
        .enumerate()
        .map(|(i, size)| {
            let prefix = format!("mixed-part-{i:02}:");
            format!("{prefix}{}", "x".repeat(size - prefix.len()))
        })
        .collect()
}

async fn pages<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    media: MediaMode,
) -> Result<Vec<ReadMessage>, String>
where
    S::Route: RunnerRoute,
{
    let mut all = Vec::new();
    let mut from = 0;
    for _ in 0..16 {
        let request = ReadRequest::range(from).with_media(media);
        let page: ReadPage = decode(
            "media read",
            expect_ok(
                "media read",
                call(
                    route,
                    ops::SESSION_READ,
                    serde_json::to_value(request).unwrap(),
                )
                .await?,
            )?,
        )?;
        for message in &page.messages {
            if media == MediaMode::Descriptors {
                let keys = case
                    .subject
                    .read_parts(&message.message)
                    .into_iter()
                    .map(|p| p.0)
                    .collect::<Vec<_>>();
                message
                    .check_descriptors(&keys)
                    .map_err(|e| format!("descriptor page decode: {e}"))?;
            }
        }
        all.extend(page.messages);
        match page.next_from_ordinal {
            Some(next) if next > from => from = next,
            None => return Ok(all),
            _ => return Err("media read made no progress".into()),
        }
    }
    Err("media pages did not terminate".into())
}

fn refs(value: &Value, out: &mut Vec<Value>) {
    if value["type"].as_str().is_some_and(|s| s.ends_with("_ref")) {
        out.push(value.clone());
    } else if let Some(map) = value.as_object() {
        for child in map.values() {
            refs(child, out);
        }
    } else if let Some(array) = value.as_array() {
        for child in array {
            refs(child, out);
        }
    }
}

fn pairing(inline: &Value, described: &Value) -> Check {
    if let Some(map) = inline.as_object() {
        for (key, value) in map {
            if [
                "tool_use_id",
                "call_key",
                "id",
                "is_error",
                "name",
                "signature",
            ]
            .contains(&key.as_str())
                && described.get(key) != Some(value)
            {
                return Err(format!("descriptors lost non-payload pairing field {key}"));
            }
            if let Some(other) = described.get(key) {
                pairing(value, other)?;
            }
        }
    } else if let (Some(a), Some(b)) = (inline.as_array(), described.as_array()) {
        for (a, b) in a.iter().zip(b) {
            pairing(a, b)?;
        }
    }
    Ok(())
}

pub async fn run<S>(case: &Case<'_, S>, name: &str) -> Result<Ending, String>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if name == "media_descriptors_model_view_refused" {
        let session = case
            .open("descriptor-model-view", Script { turns: vec![] })
            .await?;
        let error = expect_error(
            "model-view descriptors",
            call(
                &session.route,
                ops::SESSION_READ,
                json!({"view":"model","media":"descriptors"}),
            )
            .await?,
        )?;
        expect_field(
            "model-view descriptors",
            &error,
            errors::INVALID_PARAMS,
            "media",
        )?;
        return Ok(Ending::Passed);
    }
    let frame = name == "media_descriptors_frame_budget";
    let head = name == "media_descriptors_utf8_head";
    let marker = case.mint.next("descriptor");
    let mut bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a8ioAAAAASUVORK5CYII=").unwrap();
    if frame {
        bytes.resize(3_200_000, 0);
    }
    let Some(image) =
        case.subject
            .media_tool_result(&bytes, name != "media_descriptors_image_bytes", name)
    else {
        return Ok(Ending::Inapplicable(
            "the adapter cannot supply a byte-backed image result".into(),
        ));
    };
    let text = format!("{}🦀{}", "a".repeat(4095), "z".repeat(PART_INLINE_MAX));
    let mut tool = case.mint.call("descriptor-tool");
    tool.result = image;
    if name == "media_descriptors_stable_across_pages" {
        tool.result = case
            .subject
            .multipart_tool_result(&vec!["x".repeat(250 * 1024); 20])
            .ok_or("the media adapter cannot supply multipart results")?;
    }
    if frame {
        tool.arguments["oversize"] = json!("x".repeat(4_194_304));
    }
    let mut calls = vec![ScriptedPart::ToolCall(tool)];
    if frame {
        let Some(result) = case.subject.multipart_tool_result(&mixed_fallback_texts()) else {
            return Ok(Ending::Inapplicable(
                "the adapter cannot supply a mixed-size multipart result".into(),
            ));
        };
        let mut mixed = case.mint.call("mixed-fallback-tool");
        mixed.result = result;
        calls.push(ScriptedPart::ToolCall(mixed));
    }
    let turns = vec![
        ScriptedTurn::new(calls),
        ScriptedTurn::new(vec![ScriptedPart::Text(if head {
            text.clone()
        } else {
            marker.clone()
        })]),
    ];
    let mut session = case.open("media", Script { turns }).await?;
    let send_id = case.mint.next("media-send");
    let params = case.params(&session, "media prompt", &send_id, None);
    let send: SendReply = decode(
        "media send",
        expect_ok(
            "media send",
            call(&session.route, ops::SESSION_SEND, params).await?,
        )?,
    )?;
    session.first = Some((send_id, "media prompt".into()));
    wait_run_end(case.subject, &session.route, case.declared, None, &send).await?;
    let described = pages(case, &session.route, MediaMode::Descriptors).await?;
    let mut found = Vec::new();
    for message in &described {
        refs(&message.message, &mut found);
    }
    match name {
        "media_descriptors_stable_across_pages" => {
            let reference_count = found.iter().filter(|r| r["type"] == "text_ref").count();
            if reference_count != 8 {
                return Err(format!("fixed message budget must describe exactly 8 tied parts, got {reference_count}"));
            }
            let mut again = Vec::new();
            for original in &described {
                let page: ReadPage = decode("one-message page", expect_ok("one-message page", call(&session.route, ops::SESSION_READ, json!({"media":"descriptors","from_ordinal":original.ordinal,"limit":1,"max_bytes":1024})).await?)?)?;
                again.extend(page.messages);
            }
            if again != described {
                return Err("descriptor selection changed with page position or max_bytes".into());
            }
        }
        "media_descriptors_image_bytes" | "media_descriptors_nested_media" => {
            let media: Vec<_> = found.iter().filter(|r| r["type"] == "media_ref").collect();
            if media.len() != 1
                || media[0]["sha256"] != sha256_hex(&bytes)
                || media[0]["byte_size"] != bytes.len() as u64
            {
                return Err(format!(
                    "descriptors must serve media_ref over decoded bytes, not base64: {media:?}"
                ));
            }
            if name.ends_with("nested_media") && media[0].get("part").is_some() {
                return Err("nested media_ref has its own part key".into());
            }
        }
        "media_descriptors_frame_budget" => {
            if !found.iter().any(|r| r["type"] == "args_ref") {
                return Err("over-frame arguments were not described".into());
            }
            let mut selected: Vec<usize> = found
                .iter()
                .filter(|r| r["type"] == "text_ref")
                .filter_map(|r| {
                    r["head"]
                        .as_str()?
                        .strip_prefix("mixed-part-")?
                        .split(':')
                        .next()?
                        .parse()
                        .ok()
                })
                .collect();
            selected.sort_unstable();
            if selected != [1, 3] {
                return Err(format!("largest-first fallback must describe mixed parts [1, 3], got {selected:?}; the later tied part 5 stays inline"));
            }
            let huge = described
                .iter()
                .find(|m| {
                    let mut refs_in_message = Vec::new();
                    refs(&m.message, &mut refs_in_message);
                    refs_in_message.iter().any(|r| r["type"] == "args_ref")
                })
                .ok_or("missing described argument message")?;
            let request = ReadRequest::range(huge.ordinal)
                .with_media(MediaMode::Inline)
                .with_max_bytes(1_048_576);
            let reply = call(
                &session.route,
                ops::SESSION_READ,
                serde_json::to_value(request).unwrap(),
            )
            .await?;
            expect_code(
                "inline frame refusal",
                &expect_error("inline frame refusal", reply)?,
                errors::REPLY_TOO_LARGE,
            )?;
        }
        "media_descriptors_utf8_head" => {
            let reference = found
                .iter()
                .find(|r| r["type"] == "text_ref")
                .ok_or("large text missing text_ref")?;
            if reference["head"] != "a".repeat(4095)
                || reference["char_count"] != text.chars().count() as u64
                || reference["sha256"] != sha256_hex(text.as_bytes())
            {
                return Err("text_ref head must stop before the character crossing byte 4096 and hash exposed UTF-8 bytes".into());
            }
        }
        "media_descriptors_inline_unchanged"
        | "media_descriptors_full_content_digests"
        | "media_descriptors_pairing"
        | "media_descriptors_part_keys" => {
            let inline = pages(case, &session.route, MediaMode::Inline).await?;
            for old in &inline {
                let new = described
                    .iter()
                    .find(|m| m.mid == old.mid)
                    .ok_or("descriptor read lost a message")?;
                if name == "media_descriptors_full_content_digests" {
                    if new.digest.as_deref() != Some(content_digest(&old.message).as_str()) {
                        return Err(
                            "message digest hashed descriptors rather than full inline content"
                                .into(),
                        );
                    }
                    let expected = case
                        .subject
                        .read_parts(&old.message)
                        .into_iter()
                        .map(|(part, value)| {
                            cortexkit_role_llm_runner::read::descriptors::PartDigest::new(
                                part, &value,
                            )
                        })
                        .collect::<Vec<_>>();
                    if new.part_digests.as_ref() != Some(&expected) {
                        return Err(
                            "part digest hashed descriptors rather than full inline content".into(),
                        );
                    }
                }
                if name == "media_descriptors_pairing" {
                    pairing(&old.message, &new.message)?;
                }
            }
            if name == "media_descriptors_inline_unchanged" {
                let default: ReadPage = decode(
                    "default read",
                    expect_ok(
                        "default read",
                        call(&session.route, ops::SESSION_READ, json!({"from_ordinal":0})).await?,
                    )?,
                )?;
                if default.messages != inline {
                    return Err("default inline read changed".into());
                }
                for op in [ops::SESSION_READ, ops::SESSION_RESOLVE] {
                    let params = if op == ops::SESSION_READ {
                        json!({"media":"bogus"})
                    } else {
                        json!({"tag":"§1§","media":"bogus"})
                    };
                    let error =
                        expect_error("unknown media", call(&session.route, op, params).await?)?;
                    expect_code("unknown media", &error, errors::INVALID_PARAMS)?;
                    expect_field("unknown media", &error, errors::INVALID_PARAMS, "media")?;
                }
            }
        }
        _ => return Err(format!("unknown media case {name}")),
    }
    Ok(Ending::Passed)
}
