//! Descriptor reads observed on the runner route, with actual media bytes
//! supplied by the adapter and digests computed from independent stored-content
//! reads (blocks on attachment runners, inline on older runners).
use crate::{
    cases::{Case, Ending},
    drive::{call, decode, expect_code, expect_error, expect_field, expect_ok, wait_run_end},
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
};
use cortexkit_role_llm_runner::{
    content_digest::{content_digest, sha256_hex},
    errors, ops,
    read::{descriptors::PART_INLINE_MAX, MediaMode, ReadMessage, ReadPage, ReadRequest},
    send::SendReply,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

type Check = Result<(), String>;

/// Decoded size of the frame-budget case's image. Its base64 form
/// (3,200,000 characters) still fits one daemon-delivered provider reply.
const FRAME_IMAGE_BYTES: usize = 2_400_000;

/// Size of the `oversize` argument of each of the frame-budget case's two
/// sibling calls, separate from the clean image call. The two payloads alone
/// total 4,400,000 bytes, exceeding the daemon's 4,194,304-byte frame by 205,696
/// bytes before call wrappers. Inline reads must refuse that assistant message;
/// descriptors replace both arguments with small refs, so the message fits.
const FRAME_ARGUMENT_BYTES: usize = 2_200_000;

/// Number and size of the equal assistant text parts in the stable-across-pages
/// case: 3,584,000 bytes in one message, over the descriptor message budget.
const STABLE_PARTS: usize = 14;
const STABLE_PART_BYTES: usize = 250 * 1024;

/// Size of the `oversize` argument of the pairing case's sibling call: over
/// `PART_INLINE_MAX`, so a descriptors read describes the call's arguments
/// as an `args_ref`, yet well within one daemon frame.
const PAIRING_ARGUMENT_BYTES: usize = PART_INLINE_MAX + 4096;

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

/// A 1×1 grey-and-alpha PNG, base64-encoded: signature, IHDR, one IDAT and
/// IEND, each chunk with a correct CRC-32. Hosts that decode every image a
/// tool returns (OpenCode decodes with photon) drop an image that fails to
/// decode, so every byte of it must be a valid PNG.
const PNG_1X1_BASE64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+ip1sAAAAASUVORK5CYII=";

/// The chunk type that pads the frame-budget image. Lowercase first letter:
/// ancillary, so a decoder that does not know it skips it; lowercase second:
/// private; uppercase third: as the PNG specification requires; lowercase
/// fourth: safe to copy.
const PADDING_CHUNK: &[u8; 4] = b"ckPd";

/// Bytes of a chunk around its data: length, type and CRC, four each.
const CHUNK_OVERHEAD: usize = 12;

fn png_1x1() -> Vec<u8> {
    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, PNG_1X1_BASE64)
        .expect("the 1x1 PNG is valid base64")
}

/// The 1×1 PNG grown to exactly `total` bytes by a zero-filled
/// [`PADDING_CHUNK`] inserted before IEND, with its own correct CRC, so the
/// image stays a well-formed PNG with nothing after IEND.
pub(crate) fn padded_png(total: usize) -> Vec<u8> {
    let png = png_1x1();
    let iend = png.len() - CHUNK_OVERHEAD;
    let length = total - png.len() - CHUNK_OVERHEAD;
    let mut chunk = Vec::with_capacity(length + CHUNK_OVERHEAD);
    chunk.extend_from_slice(
        &u32::try_from(length)
            .expect("padding fits a chunk")
            .to_be_bytes(),
    );
    chunk.extend_from_slice(PADDING_CHUNK);
    chunk.resize(chunk.len() + length, 0);
    let crc = crc32(&chunk[4..]);
    chunk.extend_from_slice(&crc.to_be_bytes());
    let mut image = png[..iend].to_vec();
    image.extend_from_slice(&chunk);
    image.extend_from_slice(&png[iend..]);
    image
}

/// CRC-32 as PNG chunks carry it (ISO 3309: reflected polynomial
/// 0xEDB88320, initial and final XOR 0xFFFFFFFF), over a chunk's type and
/// data.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// Model text and tool-provider payloads scripted by a media case. The run and
/// provider guards share the fixture so they measure the same scripted payloads.
struct Fixture {
    /// Decoded bytes of the image result.
    image: Vec<u8>,
    /// Separate text parts in one assistant message. Their aggregate can exceed
    /// the descriptor budget without exceeding the tool-result text limit.
    assistant: Option<Vec<String>>,
    /// The `oversize` argument of each call in the image call's step whose
    /// arguments are too large to stay inline in a descriptors read. Each
    /// goes on a sibling call of its own, never on the image call: a runner
    /// may refuse large tool inputs before dispatch without losing the image.
    oversize: Vec<Value>,
    /// Top-level argument members of the first oversize sibling besides
    /// `marker` and `oversize`. The pairing case names them `id` and `name`, as
    /// tool parameters often are. They are payload, not the outer fields that
    /// identify a call and match its result, so descriptors may replace the
    /// whole arguments object with an `args_ref`.
    arguments: Vec<(&'static str, Value)>,
}

fn fixture(name: &str) -> Fixture {
    let frame = name == "media_descriptors_frame_budget";
    let pairing = name == "media_descriptors_pairing";
    let image = if frame {
        padded_png(FRAME_IMAGE_BYTES)
    } else {
        png_1x1()
    };
    Fixture {
        image,
        assistant: match name {
            name if name.starts_with("media_refs_") => Some(vec!["t".repeat(PART_INLINE_MAX + 1)]),
            "media_descriptors_stable_across_pages" => {
                Some(vec!["x".repeat(STABLE_PART_BYTES); STABLE_PARTS])
            }
            "media_descriptors_frame_budget" => Some(mixed_fallback_texts()),
            _ => None,
        },
        // For frame_budget, use two oversized siblings in one step instead of
        // one over-frame argument. Each provider request fits the daemon frame,
        // while the assistant message holding both and the image call does not.
        oversize: if name.starts_with("media_refs_") {
            vec![json!("a".repeat(PART_INLINE_MAX + 1))]
        } else if frame {
            vec![json!("x".repeat(FRAME_ARGUMENT_BYTES)); 2]
        } else if pairing {
            vec![json!("x".repeat(PAIRING_ARGUMENT_BYTES))]
        } else {
            vec![]
        },
        arguments: if pairing {
            vec![
                ("id", json!("argument-id")),
                ("name", json!("argument-name")),
            ]
        } else {
            vec![]
        },
    }
}

async fn pages<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    media: MediaMode,
    provider_profile: Option<&str>,
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
                if let Some(profile) = provider_profile {
                    message
                        .check_descriptor_rows(profile)
                        .map_err(|e| format!("provider descriptor rows: {e}"))?;
                    continue;
                }
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

/// Every call and result pairing field (and reasoning signature) of
/// `inline` is kept, unchanged, at the same place in `described`. A value
/// `described` replaced with a `*_ref` stands for the whole payload, so the
/// walk does not descend into it: a member named `id` or `name` inside a
/// call's arguments is payload, not pairing. Pairing fields beside the ref,
/// on the part that holds it, are still compared.
fn pairing(inline: &Value, described: &Value) -> Check {
    if described["type"]
        .as_str()
        .is_some_and(|kind| kind.ends_with("_ref"))
    {
        return Ok(());
    }
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
    let head = name == "media_descriptors_utf8_head";
    let marker = case.mint.next("descriptor");
    let fixture = fixture(name);
    let bytes = fixture.image;
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
    let mut calls = vec![ScriptedPart::ToolCall(tool)];
    if matches!(
        name,
        "media_descriptors_part_keys" | "media_descriptors_provider_rows"
    ) {
        calls.insert(0, ScriptedPart::Text("before the untagged call".into()));
        calls.push(ScriptedPart::Text("after the untagged call".into()));
    }
    for (index, argument) in fixture.oversize.into_iter().enumerate() {
        let mut sibling = case.mint.call(&format!("descriptor-sibling-tool-{index}"));
        sibling.arguments["oversize"] = argument;
        if index == 0 {
            for (key, value) in &fixture.arguments {
                sibling.arguments[*key] = value.clone();
            }
        }
        calls.push(ScriptedPart::ToolCall(sibling));
    }
    // Fallback selection is per message, so keep the below-threshold payloads
    // together as model text, not as an oversized provider result that §11.3.1
    // would replace with tool_result_too_large (or a host would join into one part).
    let mut answer = match fixture.assistant {
        Some(texts) => texts.into_iter().map(ScriptedPart::Text).collect(),
        None => vec![ScriptedPart::Text(if head {
            text.clone()
        } else {
            marker.clone()
        })],
    };
    if name.starts_with("media_refs_") {
        answer.push(ScriptedPart::Reasoning("r".repeat(PART_INLINE_MAX + 1)));
    }
    let turns = vec![ScriptedTurn::new(calls), ScriptedTurn::new(answer)];
    let provider_rows = name == "media_descriptors_provider_rows";
    let mut session = case
        .open(
            if provider_rows {
                "descriptor-provider"
            } else {
                "media"
            },
            Script { turns },
        )
        .await?;
    let profile = if provider_rows {
        let Some(profile) = case.subject.serializer_profile(&session.name) else {
            return Ok(Ending::Inapplicable(
                "the adapter does not name a serializer profile".into(),
            ));
        };
        Some(profile)
    } else {
        None
    };
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
    let described = pages(
        case,
        &session.route,
        MediaMode::Descriptors,
        profile.as_deref(),
    )
    .await?;
    let mut found = Vec::new();
    for message in &described {
        refs(&message.message, &mut found);
    }
    if name.starts_with("media_refs_") {
        return check_media_refs(case, &session.route, name, &described).await;
    }
    match name {
        "media_descriptors_provider_rows" => {
            if !described
                .iter()
                .any(|message| message.message.to_string().contains(&marker))
            {
                return Err("provider descriptor read lost the scripted answer".into());
            }
        }
        "media_descriptors_stable_across_pages" => {
            let reference_count = found.iter().filter(|r| r["type"] == "text_ref").count();
            // 14 x 256,000 = 3,584,000 bytes against DESCRIPTOR_MESSAGE_MAX
            // 3,145,728; each described part saves about 251,700 bytes, so 2.
            if reference_count != 2 {
                return Err(format!("fixed message budget must describe exactly 2 tied parts, got {reference_count}"));
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
            // Stored call arguments must be described even when the runner
            // refuses a sibling before dispatch and stores an error result.
            let described_arguments = |refs: &[Value]| {
                refs.iter()
                    .filter(|r| {
                        r["type"] == "args_ref"
                            && r["byte_size"]
                                .as_u64()
                                .is_some_and(|size| size >= FRAME_ARGUMENT_BYTES as u64)
                    })
                    .filter_map(|r| r["sha256"].as_str())
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
            };
            let count = described_arguments(&found);
            if count != 2 {
                return Err(format!("each of the two sibling calls' oversize arguments must be described as an args_ref, got {count} distinct"));
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
                    described_arguments(&refs_in_message) == 2
                })
                .ok_or("the two sibling calls of one step must be one assistant message whose descriptors hold both args_refs")?;
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
            let mode = if case.declared.contains(&Capability::Attachments)
                && matches!(
                    name,
                    "media_descriptors_full_content_digests" | "media_descriptors_pairing"
                ) {
                MediaMode::Blocks
            } else {
                MediaMode::Inline
            };
            let baseline = pages(case, &session.route, mode, None).await?;
            for old in &baseline {
                let new = described
                    .iter()
                    .find(|m| m.mid == old.mid)
                    .ok_or("descriptor read lost a message")?;
                if name == "media_descriptors_full_content_digests" {
                    if new.digest.as_deref() != Some(content_digest(&old.message).as_str()) {
                        return Err("message digest does not identify stored content".into());
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
                        return Err("part digest does not identify stored content".into());
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
                if default.messages != baseline {
                    return Err("default inline read changed".into());
                }
                for op in [ops::SESSION_READ] {
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

type ToolOutputs = BTreeMap<(String, String), Value>;

fn partition_media_refs<S: LlmRunnerSubject>(
    subject: &S,
    messages: &[ReadMessage],
) -> Result<(Vec<Value>, ToolOutputs), String> {
    let mut outside = Vec::new();
    let mut outputs = BTreeMap::new();
    for message in messages {
        let mut body = message.message.clone();
        for (key, _) in subject.read_parts(&message.message) {
            let Some(pointer) = subject.media_refs_tool_output_pointer(&message.message, &key)
            else {
                continue;
            };
            // The adapter identifies only an output, never the whole message.
            // Nulling that exact subtree leaves every surrounding ref checked.
            if pointer.is_empty() {
                return Err("media_refs output pointer identifies the whole message".into());
            }
            if outputs.keys().any(|(mid, previous): &(String, String)| {
                mid == &message.mid
                    && (previous == &pointer
                        || previous.starts_with(&format!("{pointer}/"))
                        || pointer.starts_with(&format!("{previous}/")))
            }) {
                return Err("media_refs output pointers overlap or repeat".into());
            }
            let output = body
                .pointer_mut(&pointer)
                .ok_or("media_refs output pointer is missing or overlaps another output")?;
            let value = std::mem::replace(output, Value::Null);
            if outputs
                .insert((message.mid.clone(), pointer), value)
                .is_some()
            {
                return Err("media_refs output pointer is repeated".into());
            }
        }
        refs(&body, &mut outside);
    }
    Ok((outside, outputs))
}

fn check_tool_output_refs(served: &Value, inline: &Value, described: &Value) -> Check {
    let mut actual = Vec::new();
    let mut expected = Vec::new();
    refs(served, &mut actual);
    refs(described, &mut expected);
    expected.retain(|value| value["type"] == "media_ref");
    for reference in actual.iter().filter(|value| value["type"] == "media_ref") {
        let Some(index) = expected.iter().position(|descriptor| {
            ["sha256", "byte_size", "kind"]
                .iter()
                .all(|field| reference[*field] == descriptor[*field])
        }) else {
            return Err(
                "tool-output media_ref differs from descriptors' sha256, byte_size or kind".into(),
            );
        };
        expected.remove(index);
    }
    if !expected.is_empty()
        && serde_json::to_vec(served).unwrap() != serde_json::to_vec(inline).unwrap()
    {
        return Err("tool output omitted media_ref but differs from inline".into());
    }
    Ok(())
}

fn check_outside_refs(name: &str, actual: &[Value], expected: &[Value], has_media: bool) -> Check {
    match name {
        "media_refs_media_are_refs"
            if !has_media
                || actual.iter().filter(|v| v["type"] == "media_ref").count() != expected.len() =>
        {
            Err("media_refs returned bytes or markers instead of media_ref".into())
        }
        "media_refs_descriptor_digests" if actual != expected => {
            Err("media_refs metadata/digests differ from descriptors".into())
        }
        _ => Ok(()),
    }
}

async fn check_media_refs<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    route: &S::Route,
    name: &str,
    described: &[ReadMessage],
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let referenced = pages(case, route, MediaMode::MediaRefs, None).await?;
    let inline = pages(case, route, MediaMode::Inline, None).await?;
    let mut all_actual = Vec::new();
    let mut all_expected = Vec::new();
    for message in &referenced {
        refs(&message.message, &mut all_actual);
    }
    for message in described {
        refs(&message.message, &mut all_expected);
    }
    let (actual, served_outputs) = partition_media_refs(case.subject, &referenced)?;
    let (mut expected, described_outputs) = partition_media_refs(case.subject, described)?;
    let (_, inline_outputs) = partition_media_refs(case.subject, &inline)?;
    let output_keys: BTreeSet<_> = described_outputs
        .keys()
        .chain(served_outputs.keys())
        .collect();
    for key in output_keys {
        check_tool_output_refs(
            served_outputs
                .get(key)
                .ok_or("media_refs omitted a tool output")?,
            inline_outputs
                .get(key)
                .ok_or("inline read omitted a tool output")?,
            described_outputs
                .get(key)
                .ok_or("descriptors omitted a tool output")?,
        )?;
    }
    expected.retain(|value| value["type"] == "media_ref");
    check_outside_refs(
        name,
        &actual,
        &expected,
        all_expected
            .iter()
            .any(|value| value["type"] == "media_ref"),
    )?;
    match name {
        "media_refs_media_are_refs" => {}
        "media_refs_large_payloads_inline" => {
            if all_actual.iter().any(|v| v["type"] != "media_ref") {
                return Err("media_refs described a non-media payload".into());
            }
            let normal = serde_json::to_string(&inline).unwrap();
            for ch in ['t', 'a', 'r'] {
                let payload = ch.to_string().repeat(PART_INLINE_MAX + 1);
                if !normal.contains(&payload) {
                    return Err(format!(
                        "inline fixture did not expose the full {ch} payload"
                    ));
                }
            }
            fn check_payloads(inline: &Value, served: Option<&Value>) -> Result<(), String> {
                match inline {
                    Value::String(text) if text.len() > PART_INLINE_MAX => {
                        if served != Some(inline) {
                            return Err("media_refs changed a large inline payload at its original location".into());
                        }
                    }
                    Value::Object(object) => {
                        for (key, child) in object {
                            check_payloads(child, served.and_then(|value| value.get(key)))?;
                        }
                    }
                    Value::Array(array) => {
                        for (index, child) in array.iter().enumerate() {
                            check_payloads(child, served.and_then(|value| value.get(index)))?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            for message in &inline {
                let served = referenced
                    .iter()
                    .find(|served| served.mid == message.mid)
                    .ok_or("media_refs omitted an inline message")?;
                check_payloads(&message.message, Some(&served.message))?;
            }
        }
        "media_refs_descriptor_digests" => {
            if all_actual.iter().any(|v| v["type"] != "media_ref") {
                return Err("media_refs metadata/digests differ from descriptors".into());
            }
            for message in &referenced {
                let descriptor = described
                    .iter()
                    .find(|m| m.mid == message.mid)
                    .ok_or("media_refs lost message identity")?;
                if message.digest.is_some() && message.digest != descriptor.digest
                    || message.part_digests.is_some()
                        && message.part_digests != descriptor.part_digests
                {
                    return Err("media_refs changed stored content digests".into());
                }
            }
        }
        _ => return Err(format!("unknown media_refs case {name}")),
    }
    if case.declared.contains(&crate::Capability::HostHistory) {
        for message in &referenced {
            let history: cortexkit_role_llm_runner::history::HistoryPage = decode(
                "media_refs history", expect_ok("media_refs history", call(route, ops::SESSION_HISTORY,
                json!({"from_ordinal":message.ordinal,"limit":1,"max_bytes":1024,"media":"media_refs"})).await?)?)?;
            let entry = history
                .messages
                .first()
                .ok_or("media_refs history omitted an entry")?;
            if entry.message != message.message || entry.mid != message.mid {
                return Err("media_refs raw history differs from session.read".into());
            }
        }
    }
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    async fn check_media_cases_with_tool_input_cap(cap: Option<usize>) {
        use crate::{
            drive::Mint,
            fork_fake::{Defects, FakeSubject},
        };
        use cortexkit_role_harness::CrashDriver;
        use std::sync::atomic::Ordering;

        for (name, oversized_calls) in [
            ("media_refs_media_are_refs", 1),
            ("media_refs_large_payloads_inline", 1),
            ("media_refs_descriptor_digests", 1),
            ("media_descriptors_frame_budget", 2),
            ("media_descriptors_pairing", 1),
        ] {
            let mut fake = FakeSubject::new(Defects::default());
            fake.tool_input_cap = cap;
            fake.descriptor_fixtures = true;
            fake.capabilities
                .extend([Capability::MediaRefs, Capability::HostHistory]);
            let declared = fake.capabilities();
            let mint = Mint::default();
            let root = tempfile::tempdir().unwrap();
            let mut driver = CrashDriver::new(&fake);
            let handle = driver.spawn(root.path()).await.unwrap();
            let case = Case {
                subject: &fake,
                handle: &handle,
                declared: &declared,
                mint: &mint,
            };
            let outcome = case
                .run(name)
                .await
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert!(
                matches!(outcome, Ending::Passed),
                "{name}: media case did not pass"
            );
            assert_eq!(
                fake.tool_input_refusals.load(Ordering::SeqCst),
                if cap.is_some() { oversized_calls } else { 0 },
                "{name}: every oversized sibling must be refused before dispatch"
            );
        }
    }

    #[tokio::test]
    async fn media_cases_pass_with_capped_tool_inputs() {
        check_media_cases_with_tool_input_cap(Some(256 * 1024)).await;
    }

    #[tokio::test]
    async fn media_cases_pass_with_uncapped_tool_inputs() {
        check_media_cases_with_tool_input_cap(None).await;
    }

    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};

    /// The largest module-to-client frame the subconscious daemon delivers,
    /// header included. A runner whose module tools cross the daemon never
    /// receives a tool provider's reply larger than this (the run simply never
    /// ends), and some client SDKs refuse to send a request of this size, so
    /// every payload a scripted tool provider receives or returns in a media
    /// case must stay below it.
    const DAEMON_FRAME_MAX: usize = cortexkit_role_llm_runner::TRANSPORT_FRAME_LIMIT;

    /// Room for what wraps a payload on the way to or from a tool provider:
    /// the frame header, the request or reply envelope, ids and the host's
    /// own tool-result fields, which differ from the reference schema below.
    const ENVELOPE_ALLOWANCE: usize = 16 * 1024;

    /// Every payload a scripted tool provider receives (a call's arguments)
    /// or returns (a call's result) in the media case `name`, shaped in a
    /// reference host schema, with images as base64 content blocks. Aggregate
    /// fallback text is model output, not a provider payload.
    fn provider_payloads(name: &str) -> Vec<(String, Value)> {
        let fixture = fixture(name);
        let image = json!({"type":"image","source":{"type":"base64",
            "media_type":"image/png","data":STANDARD.encode(&fixture.image)}});
        let mut payloads = vec![(
            "image result".to_owned(),
            json!({"type":"tool_result","tool_use_id":"call","content":[image]}),
        )];
        for (i, oversize) in fixture.oversize.into_iter().enumerate() {
            let mut arguments = json!({"marker":"cf-call-0000000000-args","oversize":oversize});
            if i == 0 {
                for (key, value) in &fixture.arguments {
                    arguments[*key] = value.clone();
                }
            }
            payloads.push((format!("arguments of oversize call {i}"), arguments));
        }
        payloads
    }

    /// A call whose arguments a descriptors read replaced with an
    /// `args_ref` keeps its pairing: `id` and `name` inside the inline
    /// arguments are payload the ref stands for. A result that lost its
    /// `tool_use_id`, or a call part that lost its own `id` beside the ref,
    /// still fails by name.
    #[test]
    fn pairing_stops_at_a_ref_and_still_checks_beside_it() {
        let arguments = json!({"id": "argument-id", "name": "argument-name", "oversize": "x"});
        let args_ref = json!({"type": "args_ref", "name": "echo", "byte_size": 9, "sha256": "ab"});
        let inline = json!({"parts": [
            {"type": "tool_use", "id": "call-1", "name": "echo", "input": arguments},
            {"type": "tool_result", "tool_use_id": "call-1", "content": "done"}
        ]});
        let described = json!({"parts": [
            {"type": "tool_use", "id": "call-1", "name": "echo", "input": args_ref},
            {"type": "tool_result", "tool_use_id": "call-1", "content": "done"}
        ]});
        assert_eq!(pairing(&inline, &described), Ok(()));

        let mut lost_result = described.clone();
        lost_result["parts"][1]
            .as_object_mut()
            .unwrap()
            .remove("tool_use_id");
        let error = pairing(&inline, &lost_result).unwrap_err();
        assert!(error.contains("tool_use_id"), "{error}");

        let mut lost_call = described.clone();
        lost_call["parts"][0].as_object_mut().unwrap().remove("id");
        let error = pairing(&inline, &lost_call).unwrap_err();
        assert!(error.ends_with("field id"), "{error}");
    }

    fn media_reference() -> Value {
        json!({"type":"media_ref", "kind":"image", "mime":"image/png", "byte_size":3,
            "sha256":"a".repeat(64)})
    }

    #[test]
    fn tool_output_refs_allow_only_byte_identical_omissions() {
        let described = json!([media_reference()]);
        let inline = json!("[image omitted]\nunchanged 🦀");
        assert!(check_tool_output_refs(&inline, &inline, &described).is_ok());
        for changed in [json!("[image omitted]\nchanged 🦀"), json!([]), Value::Null] {
            assert!(check_tool_output_refs(&changed, &inline, &described)
                .unwrap_err()
                .contains("differs from inline"));
        }
        let partial = json!([media_reference()]);
        let described = json!([media_reference(), media_reference()]);
        assert!(check_tool_output_refs(&partial, &inline, &described).is_err());
    }

    #[test]
    fn tool_output_refs_match_descriptor_sha_size_kind_and_multiplicity() {
        let reference = media_reference();
        let described = json!([reference]);
        assert!(check_tool_output_refs(&described, &json!("inline"), &described).is_ok());
        for (field, value) in [
            ("sha256", json!("b".repeat(64))),
            ("byte_size", json!(4)),
            ("kind", json!("audio")),
        ] {
            let mut changed = reference.clone();
            changed[field] = value;
            assert!(
                check_tool_output_refs(&json!([changed]), &json!("inline"), &described)
                    .unwrap_err()
                    .contains("differs from descriptors")
            );
        }
        assert!(check_tool_output_refs(
            &json!([reference, reference]),
            &json!("inline"),
            &described
        )
        .is_err());
    }

    #[test]
    fn outside_tool_outputs_keep_exact_refs_and_counts() {
        let expected = vec![media_reference()];
        for (field, value) in [
            ("sha256", json!("b".repeat(64))),
            ("byte_size", json!(4)),
            ("kind", json!("audio")),
            ("mime", json!("image/jpeg")),
            ("width", json!(1)),
            ("part", json!("different-part")),
        ] {
            let mut actual = expected.clone();
            actual[0][field] = value;
            assert!(
                check_outside_refs("media_refs_descriptor_digests", &actual, &expected, true)
                    .is_err()
            );
        }
        assert!(check_outside_refs("media_refs_media_are_refs", &[], &expected, true).is_err());
        assert!(check_outside_refs(
            "media_refs_media_are_refs",
            &[expected[0].clone(), expected[0].clone()],
            &expected,
            true
        )
        .is_err());
        assert!(check_outside_refs("media_refs_media_are_refs", &[], &[], false).is_err());
    }

    /// Every problem with `png` as a PNG: its signature, each chunk's
    /// length and CRC-32, IHDR first with 13 bytes, at least one IDAT, IEND
    /// last and empty, and nothing after IEND. Empty when it is well formed.
    fn png_problems(png: &[u8]) -> Vec<String> {
        const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
        if !png.starts_with(SIGNATURE) {
            return vec!["no PNG signature".into()];
        }
        let mut problems = Vec::new();
        let mut types = Vec::new();
        let mut at = SIGNATURE.len();
        while at < png.len() {
            if types.last() == Some(b"IEND") {
                problems.push(format!("{} bytes after IEND", png.len() - at));
                break;
            }
            if png.len() - at < CHUNK_OVERHEAD {
                problems.push(format!("a truncated chunk at byte {at}"));
                break;
            }
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            let end = at + 8 + length;
            if end + 4 > png.len() {
                problems.push(format!("a chunk at byte {at} runs past the end"));
                break;
            }
            let kind: [u8; 4] = png[at + 4..at + 8].try_into().unwrap();
            let stored = u32::from_be_bytes(png[end..end + 4].try_into().unwrap());
            let computed = crc32(&png[at + 4..end]);
            if stored != computed {
                problems.push(format!(
                    "{} stores CRC {stored:#010x}, but its type and data give {computed:#010x}",
                    String::from_utf8_lossy(&kind)
                ));
            }
            if &kind == b"IHDR" && length != 13 {
                problems.push(format!("IHDR holds {length} bytes, not 13"));
            }
            if &kind == b"IEND" && length != 0 {
                problems.push(format!("IEND holds {length} bytes, not 0"));
            }
            types.push(kind);
            at = end + 4;
        }
        if types.first() != Some(b"IHDR") {
            problems.push("the first chunk is not IHDR".into());
        }
        if !types.contains(b"IDAT") {
            problems.push("no IDAT chunk".into());
        }
        if types.last() != Some(b"IEND") {
            problems.push("the last chunk is not IEND".into());
        }
        problems
    }

    /// The checksum used to judge the scripted PNGs is CRC-32 itself: the
    /// standard check value, and the IEND chunk every PNG ends with.
    #[test]
    fn png_crc32_matches_the_standard_check_values() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
    }

    /// Every PNG a media case scripts (the 1×1 image and the padded
    /// frame-budget image) is well formed: correct chunk CRCs, IHDR to
    /// IEND, and nothing after IEND. A host that decodes tool-result images
    /// drops one that is not, and the case could then never pass there.
    #[test]
    fn every_scripted_png_is_well_formed() {
        let mut checked = vec![("the 1x1 image".to_owned(), png_1x1())];
        for spec in crate::CASES
            .iter()
            .filter(|spec| spec.name.starts_with("media_"))
        {
            checked.push((spec.name.to_owned(), fixture(spec.name).image));
        }
        let padded = fixture("media_descriptors_frame_budget").image;
        assert_eq!(padded.len(), FRAME_IMAGE_BYTES);
        assert!(padded
            .windows(PADDING_CHUNK.len())
            .any(|window| window == PADDING_CHUNK));
        let mut problems = Vec::new();
        for (what, png) in &checked {
            for problem in png_problems(png) {
                problems.push(format!("{what}: {problem}"));
            }
        }
        assert!(
            problems.is_empty(),
            "malformed scripted PNGs: {problems:#?}"
        );
    }

    #[test]
    fn every_media_provider_payload_fits_a_daemon_frame() {
        let names: Vec<_> = crate::CASES
            .iter()
            .map(|spec| spec.name)
            .filter(|name| name.starts_with("media_"))
            .collect();
        assert!(
            names.contains(&"media_descriptors_frame_budget"),
            "the media cases were not found in CASES: {names:?}"
        );
        let mut over = Vec::new();
        for name in names {
            for (what, payload) in provider_payloads(name) {
                let size = serde_json::to_vec(&payload).unwrap().len() + ENVELOPE_ALLOWANCE;
                if size >= DAEMON_FRAME_MAX {
                    over.push(format!("{name}: {what} is {size} bytes with envelope"));
                }
            }
        }
        assert!(
            over.is_empty(),
            "provider payloads that cannot cross the daemon's {DAEMON_FRAME_MAX}-byte frame: {over:#?}"
        );
    }

    #[test]
    fn media_fixture_result_text_stays_within_provider_limit() {
        // These reference-schema results contain only text and image blocks.
        // For negotiated `tool-result/v1` image parts, image bytes do not count
        // toward the model-visible provider-result limit.
        fn text_bytes(value: &Value) -> usize {
            if value["type"] == "text" {
                return value["text"].as_str().unwrap().len();
            }
            match value {
                Value::Array(items) => items.iter().map(text_bytes).sum(),
                Value::Object(fields) => fields.values().map(text_bytes).sum(),
                _ => 0,
            }
        }
        let mut oversized = Vec::new();
        for name in [
            "media_descriptors_frame_budget",
            "media_descriptors_stable_across_pages",
        ] {
            for (what, payload) in provider_payloads(name) {
                if what.ends_with("result") {
                    let bytes = text_bytes(&payload);
                    if bytes > 65_536 {
                        oversized.push(format!("{name}: {what} has {bytes} text bytes"));
                    }
                }
            }
        }
        assert!(
            oversized.is_empty(),
            "oversized provider results: {oversized:#?}"
        );
    }
}
