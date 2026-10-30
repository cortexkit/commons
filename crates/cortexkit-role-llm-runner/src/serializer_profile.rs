//! Serializer profiles: names for the schema of a `session.read` entry's
//! `message` body.
//!
//! The role keeps message bodies in the runner's own schema (CONTRACT
//! §4.2). A profile names one such schema, so a consumer that knows the
//! profile can read the body. The same value reaches a session's providers
//! as their plan item's `params.serializer_profile` (pinned in the
//! step-transform-provider and compaction-provider contracts, which name
//! `opencode-aisdk`). This module names the supported profiles and pins
//! their part containers; `display` holds the supported projections to
//! display rows, and [`error_output`] the `owned-broca` tool error with its
//! module-outage members and the text every runner renders for it.

use serde_json::Value;

pub mod error_output;
pub(crate) mod pi;
pub use pi::{
    pi_session_projection, PiJournalError, PiNativeCut, PiOrdinalEntry, PiSessionProjection,
};

/// A runner serving its transcript in the Anthropic Messages request
/// format: an entry body is the message exactly as the client sent it in
/// its Messages request, with `cache_control` members removed only where
/// [`strip_cache_control`] removes them.
pub const ANTHROPIC_MESSAGES: &str = "anthropic-messages";

/// A runner serving OpenCode's own stored messages: an entry body is the
/// message as OpenCode stores it, `{info, parts}`, where `info` is the
/// message record (`id`, `role`, `time`, and on an assistant message
/// `providerID` and `modelID`, on a user message `model: {providerID,
/// modelID}`) and `parts` its parts in order, each `{id, type, ...}` (`text`,
/// `reasoning`, `tool` with `callID`, `tool` and `state: {status, input,
/// output?, error?, attachments?}`, `file` with `mime`, `url` and
/// `filename?`, and OpenCode's bookkeeping parts). It is not an AI SDK model
/// message. Read stability applies: fields OpenCode changes after a message
/// finished stay as the runner first read them.
pub const OPENCODE_AISDK: &str = "opencode-aisdk";

/// The `owned-broca` profile: an entry body stores its parts in a `content`
/// array. Descriptors replace payloads inside a part, never the whole array.
pub const OWNED_BROCA: &str = "owned-broca";

/// The `pi` profile: bodies are wrapped journal entries, not bare messages.
/// See [`pi_session_projection`] for active paths, ordinals, edits and cuts.
pub const PI: &str = "pi";

/// Count top-level parts without deriving their runner-defined keys.
/// `anthropic-messages` uses `content`, `opencode-aisdk` uses `parts`,
/// `owned-broca` uses a `content` array. `pi` counts projected blocks of wrapped
/// `message`, `custom_message` and `branch_summary` entries, including shell text.
/// Arrays count only their immediate elements. For `anthropic-messages` and
/// `pi`, a string (even empty) or a whole-content `text_ref` is one part, except
/// that a `pi` `branch_summary` with an empty `summary` projects nothing and
/// counts zero.
/// Unknown profiles and malformed part containers are refused by name.
pub fn top_level_part_count(profile: &str, message: &Value) -> Result<usize, String> {
    match profile {
        ANTHROPIC_MESSAGES => match message.get("content") {
            Some(Value::Array(parts)) => Ok(parts.len()),
            Some(Value::String(_)) => Ok(1),
            Some(Value::Object(reference))
                if reference.get("type").and_then(Value::as_str) == Some("text_ref") =>
            {
                Ok(1)
            }
            _ => {
                Err("anthropic-messages message content must be a string, array or text_ref".into())
            }
        },
        OPENCODE_AISDK => message
            .get("parts")
            .and_then(Value::as_array)
            .map(Vec::len)
            .ok_or_else(|| "opencode-aisdk message parts must be an array".into()),
        OWNED_BROCA => message
            .get("content")
            .and_then(Value::as_array)
            .map(Vec::len)
            .ok_or_else(|| "owned-broca message content must be an array".into()),
        PI => pi::projected_content(message).and_then(|content| pi::content_count(&content)),
        _ => Err(format!("unknown serializer profile: {profile}")),
    }
}

/// The member the Anthropic Messages format uses for cache markers.
pub const CACHE_CONTROL: &str = "cache_control";

/// The block types nested in a `tool_result`'s content whose cache markers
/// are removed. Any other nested block is kept byte for byte.
pub const TOOL_RESULT_MARKED_TYPES: &[&str] = &["text", "image", "document"];

fn without_marker(block: &mut Value) {
    if let Some(members) = block.as_object_mut() {
        members.remove(CACHE_CONTROL);
    }
}

/// The `anthropic-messages` entry body for `message`, a message as a client
/// sent it in a Messages request: `cache_control` is removed from each of
/// the message's top-level content blocks, whatever its type, and from the
/// `text`, `image` and `document` blocks nested in a `tool_result`'s
/// content. Nothing else changes: not a `tool_use` block's `input`, not
/// anything nested in a block of a type this function does not know, and
/// not a message whose `content` is a string.
pub fn strip_cache_control(message: &Value) -> Value {
    let mut message = message.clone();
    let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) else {
        return message;
    };
    for block in blocks {
        without_marker(block);
        if block["type"] != "tool_result" {
            continue;
        }
        if let Some(nested) = block.get_mut("content").and_then(Value::as_array_mut) {
            for inner in nested {
                let marked = inner["type"]
                    .as_str()
                    .is_some_and(|kind| TOOL_RESULT_MARKED_TYPES.contains(&kind));
                if marked {
                    without_marker(inner);
                }
            }
        }
    }
    message
}

/// The entry a runner keeps for a message it is sent, given the entry it
/// first ingested for that message, if any. Read stability (CONTRACT §4.2):
/// once ingested, the first form is served, so a client re-sending the
/// same message with its cache marker moved (a string `content` turning
/// into a one-block array, say) never changes the entry.
pub fn ingest(first: Option<&Value>, sent: &Value) -> Value {
    match first {
        Some(first) => first.clone(),
        None => strip_cache_control(sent),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn top_level_part_counts_cover_named_profiles_and_descriptor_shapes() {
        use serde_json::json;
        for (message, count) in [
            (json!({"content":""}), 1),
            (json!({"content":"hello"}), 1),
            (json!({"content":[]}), 0),
            (json!({"content":[{"type":"text","text":"one"}]}), 1),
            (
                json!({"content":[{"type":"tool_result","content":[
                {"type":"text","text":"nested"}, {"type":"image"}
            ]}, {"type":"future_part"}]}),
                2,
            ),
            (json!({"content":{"type":"text_ref","byte_size":100}}), 1),
            (
                json!({"content":[{"type":"text_ref"},{"type":"media_ref"}]}),
                2,
            ),
        ] {
            assert_eq!(
                top_level_part_count(ANTHROPIC_MESSAGES, &message),
                Ok(count)
            );
        }
        for (message, count) in [
            (json!({"info":{},"parts":[]}), 0),
            (json!({"info":{},"parts":[{"type":"text"}]}), 1),
            (
                json!({"info":{},"parts":[{"type":"tool","state":{"attachments":[{},{}]}},
                {"type":"step-start"}, {"type":"future_part"}]}),
                3,
            ),
        ] {
            assert_eq!(top_level_part_count(OPENCODE_AISDK, &message), Ok(count));
        }
    }

    #[test]
    fn top_level_part_counts_refuse_unknown_profiles_and_malformed_containers() {
        use serde_json::json;
        assert_eq!(
            top_level_part_count("future-profile", &json!({})).unwrap_err(),
            "unknown serializer profile: future-profile"
        );
        for content in [json!(null), json!(42), json!({"type":"unknown_ref"})] {
            assert_eq!(
                top_level_part_count(ANTHROPIC_MESSAGES, &json!({"content":content})).unwrap_err(),
                "anthropic-messages message content must be a string, array or text_ref"
            );
        }
        assert!(top_level_part_count(ANTHROPIC_MESSAGES, &json!({})).is_err());
        for message in [json!({}), json!({"parts":null}), json!({"parts":"text"})] {
            assert_eq!(
                top_level_part_count(OPENCODE_AISDK, &message).unwrap_err(),
                "opencode-aisdk message parts must be an array"
            );
        }
    }

    #[test]
    fn owned_broca_counts_only_content_array_parts() {
        use serde_json::json;
        for (message, count) in [
            (
                json!({"role":"user","content":[{"type":"text","text":"hello"}]}),
                1,
            ),
            (
                json!({"role":"assistant","content":[{"type":"text","text":"answer"}],
                    "origin":{"provider_module_id":"p","model_id":"m"}}),
                1,
            ),
            (
                json!({"role":"assistant","content":[{"type":"tool_call",
                    "tool_call_id":"reduce_old_outputs","tool_name":"ctx_reduce",
                    "input":{"drop":"2,5"}}],
                    "origin":{"provider_module_id":"p","model_id":"m"}}),
                1,
            ),
            (
                json!({"role":"assistant","content":[{"type":"text","text":"answer"},
                    {"type":"reasoning","text":"reason"},
                    {"type":"tool_call","tool_call_id":"c1","tool_name":"read","input":{}}],
                    "origin":{"provider_module_id":"p","model_id":"m"},
                    "tool_calls":[{"id":"c1"},{"id":"c2"}]}),
                3,
            ),
            (
                json!({"role":"user","content":[{"type":"tool_result","tool_call_id":"c1",
                    "output":[{"type":"text","text":"image"},
                        {"type":"image","data":"AAAA","mime_type":"image/png"}]}]}),
                1,
            ),
            (json!({"role":"user","content":[]}), 0),
            (
                json!({"role":"user","content":[{"type":"text","text":{"type":"text_ref"}},
                    {"type":"image","data":{"type":"media_ref"}}]}),
                2,
            ),
        ] {
            assert_eq!(
                top_level_part_count(OWNED_BROCA, &message),
                Ok(count),
                "{message}"
            );
        }
    }

    #[test]
    fn owned_broca_refuses_malformed_content_by_name() {
        use serde_json::json;
        let error = "owned-broca message content must be an array";
        for content in [
            json!("hello"),
            json!(""),
            json!({"type":"text_ref","byte_size":100}),
            json!(null),
            json!(42),
            json!(false),
            json!({}),
        ] {
            assert_eq!(
                top_level_part_count(OWNED_BROCA, &json!({"role":"user","content":content}))
                    .unwrap_err(),
                error
            );
        }
        assert_eq!(
            top_level_part_count(
                OWNED_BROCA,
                &json!({"role":"assistant",
                "origin":{"provider_module_id":"p","model_id":"m"},"tool_calls":[]})
            )
            .unwrap_err(),
            error
        );
    }

    #[test]
    fn pi_counts_only_wrapped_message_content_parts() {
        use serde_json::json;
        for (message, count) in [
            (
                json!({"type":"message","id":"a1","parentId":null,
                    "message":{"role":"user","content":"hello"}}),
                1,
            ),
            (
                json!({"type":"message","id":"a2","parentId":"a1",
                    "message":{"role":"user","content":[{"type":"text","text":"see"},
                        {"type":"image","data":"AAAA","mimeType":"image/png"}]}}),
                2,
            ),
            (
                json!({"type":"message","id":"a3","parentId":"a2",
                    "message":{"role":"assistant","content":[{"type":"thinking","thinking":"..."},
                        {"type":"text","text":"ok"},
                        {"type":"toolCall","id":"c1","name":"read","arguments":{}}]}}),
                3,
            ),
            (
                json!({"type":"message","id":"a4","parentId":"a3",
                    "message":{"role":"toolResult","toolCallId":"c1","toolName":"read",
                        "content":[{"type":"text","text":"..."}],"isError":false,"details":{"x":1}}}),
                1,
            ),
            (
                json!({"type":"message","id":"a5","parentId":"a4",
                    "message":{"role":"user","content":""}}),
                1,
            ),
            (
                json!({"type":"message","id":"a6","parentId":"a5",
                    "message":{"role":"user","content":{"type":"text_ref","byte_size":100}}}),
                1,
            ),
            (
                json!({"type":"message","id":"a7","parentId":"a6",
                    "message":{"role":"user","content":[]}}),
                0,
            ),
        ] {
            assert_eq!(top_level_part_count(PI, &message), Ok(count), "{message}");
        }
    }

    #[test]
    fn pi_refuses_bare_bodies_and_malformed_content_by_name() {
        use serde_json::json;
        let error = "pi wrapped message content must be a string, array or text_ref";
        for content in [
            json!(null),
            json!(42),
            json!(false),
            json!({}),
            json!({"type":"unknown_ref"}),
            json!({"type":"media_ref"}),
            json!({"type":42}),
        ] {
            assert_eq!(
                top_level_part_count(
                    PI,
                    &json!({"type":"message","id":"a1","parentId":null,
                    "message":{"role":"user","content":content}})
                )
                .unwrap_err(),
                error
            );
        }
        for message in [
            json!({}),
            json!({"role":"user","content":"hello"}),
            json!({"role":"user","content":[]}),
            json!({"role":"user","content":{"type":"text_ref"}}),
            json!({"type":"message","message":null}),
            json!({"type":"message","message":"hello"}),
            json!({"type":"message","message":[]}),
            json!({"type":"message","message":{"role":"user"},"content":"outside"}),
        ] {
            assert_eq!(
                top_level_part_count(PI, &message).unwrap_err(),
                error,
                "{message}"
            );
        }
    }

    #[test]
    fn markers_are_removed_only_where_the_profile_allows() {
        let file = vectors::load("serializer-anthropic-messages.json");
        for case in vectors::cases(&file, "strip") {
            let name = case["name"].as_str().unwrap();
            assert_eq!(strip_cache_control(&case["sent"]), case["entry"], "{name}");
            // Stripping is idempotent: an entry is already in its final form.
            assert_eq!(strip_cache_control(&case["entry"]), case["entry"], "{name}");
        }
        let kept = file["strip"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|case| {
                case["entry"]["content"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
            })
            .any(|block| {
                block["type"] == "tool_use" && block["input"].to_string().contains(CACHE_CONTROL)
            });
        assert!(kept, "no vector keeps a marker inside tool_use input");
    }

    #[test]
    fn a_resend_with_a_moved_marker_keeps_the_first_form() {
        let file = vectors::load("serializer-anthropic-messages.json");
        for case in vectors::cases(&file, "resends") {
            let name = case["name"].as_str().unwrap();
            let first = ingest(None, &case["first"]);
            assert_eq!(first, case["entry"], "{name}");
            let again = ingest(Some(&first), &case["resent"]);
            assert_eq!(again, case["entry"], "{name}");
            assert_ne!(
                strip_cache_control(&case["resent"]),
                case["entry"],
                "{name}: the resend differs"
            );
        }
        assert_eq!(file["profile"], ANTHROPIC_MESSAGES);
    }
}
