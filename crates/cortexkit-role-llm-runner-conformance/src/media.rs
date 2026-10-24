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

type Check = Result<(), String>;

/// Decoded size of the frame-budget case's image. Its base64 form
/// (3,200,000 characters) still fits one daemon-delivered provider reply.
const FRAME_IMAGE_BYTES: usize = 2_400_000;

/// Size of the `oversize` argument of each of the frame-budget case's two
/// sibling calls. Each call's request fits the daemon frame on its own, while
/// the assistant message holding both calls is over the daemon's 4 MiB frame,
/// so an inline read of that message must be refused.
const FRAME_ARGUMENT_BYTES: usize = 2_200_000;

/// Number and size of the equal text parts in the stable-across-pages case's
/// result: 3,584,000 bytes, over the descriptor message budget yet within one
/// daemon-delivered provider reply.
const STABLE_PARTS: usize = 14;
const STABLE_PART_BYTES: usize = 250 * 1024;

/// Size of the `oversize` argument of the pairing case's call: over
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

/// What a media case hands the scripted tool provider, before the adapter
/// shapes the results in its host's schema. The run and the frame guard test
/// both read it, so the guard measures exactly what a case scripts.
struct Fixture {
    /// Decoded bytes of the image result.
    image: Vec<u8>,
    /// Texts of a multipart result that replaces the image result.
    multipart: Option<Vec<String>>,
    /// The `oversize` argument of each call in the image call's step whose
    /// arguments are too large to stay inline in a descriptors read; the
    /// first goes on the image call, each later one on a sibling call of its
    /// own.
    oversize: Vec<Value>,
    /// Texts of the mixed-size fallback result scripted in the same step.
    mixed: Option<Vec<String>>,
    /// Top-level argument members of the image call besides `marker` and
    /// `oversize`. The pairing case names them `id` and `name`, as tool
    /// parameters often are: inside arguments a descriptors read replaces
    /// whole with an `args_ref`, they are payload, not pairing fields.
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
        multipart: (name == "media_descriptors_stable_across_pages")
            .then(|| vec!["x".repeat(STABLE_PART_BYTES); STABLE_PARTS]),
        // Two sibling calls in one step instead of one over-frame argument:
        // each call's request to the tool provider fits the daemon frame,
        // while the assistant message holding both does not.
        oversize: if frame {
            vec![json!("x".repeat(FRAME_ARGUMENT_BYTES)); 2]
        } else if pairing {
            vec![json!("x".repeat(PAIRING_ARGUMENT_BYTES))]
        } else {
            vec![]
        },
        mixed: frame.then(mixed_fallback_texts),
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
    if let Some(texts) = &fixture.multipart {
        tool.result = case
            .subject
            .multipart_tool_result(texts)
            .ok_or("the media adapter cannot supply multipart results")?;
    }
    let mut oversize = fixture.oversize.into_iter();
    if let Some(argument) = oversize.next() {
        tool.arguments["oversize"] = argument;
    }
    for (key, value) in fixture.arguments {
        tool.arguments[key] = value;
    }
    let mut calls = vec![ScriptedPart::ToolCall(tool)];
    if matches!(
        name,
        "media_descriptors_part_keys" | "media_descriptors_provider_rows"
    ) {
        calls.insert(0, ScriptedPart::Text("before the untagged call".into()));
        calls.push(ScriptedPart::Text("after the untagged call".into()));
    }
    for argument in oversize {
        let mut sibling = case.mint.call("descriptor-sibling-tool");
        sibling.arguments["oversize"] = argument;
        calls.push(ScriptedPart::ToolCall(sibling));
    }
    if let Some(texts) = &fixture.mixed {
        let Some(result) = case.subject.multipart_tool_result(texts) else {
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

#[cfg(test)]
mod tests {
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

    fn text_result(texts: &[String]) -> Value {
        json!({"type":"tool_result","tool_use_id":"call","content":texts
            .iter()
            .map(|text| json!({"type":"text","text":text}))
            .collect::<Vec<_>>()})
    }

    /// Every payload a scripted tool provider receives (a call's arguments)
    /// or returns (a call's result) in the media case `name`, shaped in a
    /// reference host schema: images as base64 content blocks, multipart
    /// results as text blocks.
    fn provider_payloads(name: &str) -> Vec<(String, Value)> {
        let fixture = fixture(name);
        let image = json!({"type":"image","source":{"type":"base64",
            "media_type":"image/png","data":STANDARD.encode(&fixture.image)}});
        let mut payloads = vec![(
            "image result".to_owned(),
            json!({"type":"tool_result","tool_use_id":"call","content":[image]}),
        )];
        if let Some(texts) = &fixture.multipart {
            payloads.push(("multipart result".to_owned(), text_result(texts)));
        }
        if let Some(texts) = &fixture.mixed {
            payloads.push(("mixed-size result".to_owned(), text_result(texts)));
        }
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
}
