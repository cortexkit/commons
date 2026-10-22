//! Serializer profiles: names for the schema of a `session.read` entry's
//! `message` body.
//!
//! The role keeps message bodies in the runner's own schema (CONTRACT
//! §4.2). A profile names one such schema, so a consumer that knows the
//! profile can read the body. The same value reaches a session's providers
//! as their plan item's `params.serializer_profile` (pinned in the
//! step-transform-provider and compaction-provider contracts, which name
//! `opencode-aisdk`). This module defines [`ANTHROPIC_MESSAGES`] and pins
//! the body schema of [`OPENCODE_AISDK`]; `display` holds each profile's
//! projection to a display row.

use serde_json::Value;

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
