//! Schema adapter for descriptor reads in the reference fake. Payload paths
//! are discovered independently of their digest rows and never remove call
//! pairing or reasoning signatures.
use super::*;
use base64::{engine::general_purpose::STANDARD, Engine};
use cortexkit_role_llm_runner::{
    content_digest::content_digest,
    read::{
        descriptors::{self, PartDigest, Payload, Reference},
        MediaMode,
    },
};

pub fn parts(message: &Value) -> Vec<(String, Value)> {
    match message["parts"].as_array() {
        Some(parts) => parts
            .iter()
            .enumerate()
            .map(|(i, p)| (i.to_string(), p.clone()))
            .collect(),
        None => vec![("0".into(), message.clone())],
    }
}

pub fn payloads(message: &Value) -> Vec<Payload> {
    let mut out = Vec::new();
    if let Some(parts) = message["parts"].as_array() {
        for (i, part) in parts.iter().enumerate() {
            collect(part, &format!("/parts/{i}"), Some(i.to_string()), &mut out);
        }
    } else {
        collect(message, "", Some("0".into()), &mut out);
    }
    out
}

fn collect(value: &Value, path: &str, key: Option<String>, out: &mut Vec<Payload>) {
    if let Some(array) = value.as_array() {
        for (i, child) in array.iter().enumerate() {
            collect(child, &format!("{path}/{i}"), None, out);
        }
        return;
    }
    let Some(map) = value.as_object() else {
        return;
    };
    let kind = value["type"].as_str().unwrap_or("");
    if kind == "media" {
        out.push(Payload {
            pointer: path.into(),
            reference: Reference::MediaRef {
                part: key,
                kind: value["kind"].as_str().unwrap().into(),
                mime: value["mime"].as_str().unwrap().into(),
                byte_size: value["bytes"].as_u64().unwrap(),
                sha256: value["sha256"].as_str().unwrap().into(),
                width: None,
                height: None,
            },
            required: true,
        });
        return;
    }
    if ["image", "document", "audio", "file"].contains(&kind) && value["source"]["type"] == "base64"
    {
        let bytes = STANDARD
            .decode(value["source"]["data"].as_str().unwrap())
            .unwrap();
        out.push(Payload {
            pointer: path.into(),
            reference: Reference::media(
                key,
                if kind == "document" { "pdf" } else { kind },
                value["source"]["media_type"]
                    .as_str()
                    .unwrap_or("application/octet-stream"),
                &bytes,
                None,
            ),
            required: true,
        });
        return;
    }
    if kind == "text" {
        if let Some(text) = value["text"].as_str() {
            out.push(Payload {
                pointer: path.into(),
                reference: Reference::text(key, text),
                required: false,
            });
            return;
        }
    }
    for (name, child) in map {
        let pointer = format!("{path}/{name}");
        if ["arguments", "input"].contains(&name.as_str())
            && ["tool_call", "tool_use"].contains(&kind)
        {
            let bytes = serde_json::to_vec(child).unwrap();
            out.push(Payload {
                pointer,
                reference: Reference::arguments(
                    None,
                    value["name"].as_str().unwrap_or("tool"),
                    &bytes,
                ),
                required: false,
            });
        } else if ["reasoning", "thinking"].contains(&kind)
            && ["text", "thinking", "data"].contains(&name.as_str())
        {
            let bytes = child
                .as_str()
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_else(|| serde_json::to_vec(child).unwrap());
            out.push(Payload {
                pointer,
                reference: Reference::reasoning(None, &bytes),
                required: false,
            });
        } else if ["text", "content", "output", "data"].contains(&name.as_str())
            && child.is_string()
            && !(name == "data" && value["type"] != "text")
        {
            out.push(Payload {
                pointer,
                reference: Reference::text(None, child.as_str().unwrap()),
                required: false,
            });
        } else {
            collect(child, &pointer, None, out);
        }
    }
}

pub fn describe_message(
    mut read: ReadMessage,
    page_budget: usize,
    defects: &Defects,
) -> Result<ReadMessage, String> {
    let inline = read.message.clone();
    read.digest = Some(content_digest(&inline));
    read.part_digests = Some(
        parts(&inline)
            .into_iter()
            .map(|(key, value)| PartDigest::new(key, &value))
            .collect(),
    );
    read.message = Value::Null;
    let envelope = serde_json::to_vec(&read).unwrap().len() - 4;
    let mut budget = descriptors::DESCRIPTOR_MESSAGE_MAX.saturating_sub(envelope);
    if defects.media == Some("page_position") && page_budget <= 1024 {
        budget = budget.saturating_sub(300_000);
    }
    let mut candidates = payloads(&inline);
    if defects.media == Some("base64") {
        candidates.retain(|p| !p.required);
    }
    if defects.media == Some("nested_base64") {
        candidates.retain(|p| !p.required || !p.pointer.contains("/content/"));
    }
    read.message = if defects.media == Some("smallest_first") {
        smallest_first(&inline, &candidates, budget)?
    } else {
        descriptors::describe(&inline, &candidates, budget)?
    };
    if defects.media == Some("head_boundary") {
        corrupt_heads(&mut read.message);
    }
    if defects.media == Some("pairing") {
        drop_pairing(&mut read.message);
    }
    let hashed = if defects.media == Some("hash_descriptor") {
        &read.message
    } else {
        &inline
    };
    read.digest = Some(content_digest(hashed));
    read.part_digests = Some(
        parts(hashed)
            .into_iter()
            .map(|(key, value)| PartDigest::new(key, &value))
            .collect(),
    );
    if defects.media == Some("part_key") {
        read.part_digests.as_mut().unwrap()[0].part = "not-a-part".into();
    }
    Ok(read)
}

/// Deliberately violate fallback ordering while retaining the normal media
/// and over-threshold rules, so the mixed-size case must observe the choice.
fn smallest_first(inline: &Value, candidates: &[Payload], budget: usize) -> Result<Value, String> {
    let mut answer = inline.clone();
    let mut optional = Vec::new();
    for candidate in candidates {
        if candidate.required
            || candidate.reference.byte_size() > descriptors::PART_INLINE_MAX as u64
        {
            *answer.pointer_mut(&candidate.pointer).unwrap() =
                serde_json::to_value(&candidate.reference).unwrap();
        } else {
            optional.push(candidate);
        }
    }
    optional.sort_by_key(|candidate| candidate.reference.byte_size());
    for candidate in optional {
        if serde_json::to_vec(&answer).unwrap().len() <= budget {
            break;
        }
        *answer.pointer_mut(&candidate.pointer).unwrap() =
            serde_json::to_value(&candidate.reference).unwrap();
    }
    if serde_json::to_vec(&answer).unwrap().len() > budget {
        return Err("descriptor message exceeds budget".into());
    }
    Ok(answer)
}

fn corrupt_heads(value: &mut Value) {
    if value["type"] == "text_ref" {
        let head = value["head"].as_str().unwrap();
        value["head"] = json!(format!("{head}�"));
    } else if let Some(m) = value.as_object_mut() {
        for v in m.values_mut() {
            corrupt_heads(v);
        }
    } else if let Some(a) = value.as_array_mut() {
        for v in a {
            corrupt_heads(v);
        }
    }
}
fn drop_pairing(value: &mut Value) {
    if value["type"] == "tool_result" {
        value.as_object_mut().unwrap().remove("tool_use_id");
    }
    if let Some(m) = value.as_object_mut() {
        for v in m.values_mut() {
            drop_pairing(v);
        }
    } else if let Some(a) = value.as_array_mut() {
        for v in a {
            drop_pairing(v);
        }
    }
}

pub fn page(page: ReadPage, mode: Option<MediaMode>, budget: usize, defects: &Defects) -> Reply {
    let mut page = page;
    if mode == Some(MediaMode::Descriptors) {
        let messages = page
            .messages
            .into_iter()
            .map(|m| {
                let ordinal = m.ordinal;
                describe_message(m, budget, defects).map_err(|_| ordinal)
            })
            .collect::<Result<Vec<_>, _>>();
        page.messages = match messages {
            Ok(messages) => messages,
            Err(ordinal) => {
                return refuse(errors::REPLY_TOO_LARGE, Some(json!({"ordinal":ordinal})))
            }
        };
    }
    if mode != Some(MediaMode::Descriptors) {
        attachments::page(&mut page, mode, defects);
    }
    let value = serde_json::to_value(page).unwrap();
    if mode != Some(MediaMode::Descriptors)
        && serde_json::to_vec(&value).unwrap().len() + 21
            > cortexkit_role_llm_runner::TRANSPORT_FRAME_LIMIT
    {
        return refuse(errors::REPLY_TOO_LARGE, None);
    }
    Reply::Response(value)
}
