//! The `llm-runner/v1` test vectors, compiled in.
//!
//! The files live in this crate's `test-vectors/llm-runner-v1/` and ship in
//! its package. With the `test-vectors` feature this module is public, so a
//! runner or consumer that depends on this crate by version checks exactly
//! the bytes this crate's own tests check, without a checkout of this
//! repository. [`SHA256SUMS`] pins every file's bytes; [`README`] describes
//! each file's cases and the tests that read them.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "llm-runner-v1";

/// Every vector file as `(file name, contents)`, in the order
/// [`SHA256SUMS`] lists them.
pub const FILES: &[(&str, &str)] = &[
    (
        "app-readers/admission.json",
        include_str!("../test-vectors/llm-runner-v1/app-readers/admission.json"),
    ),
    (
        "app-readers/README.md",
        include_str!("../test-vectors/llm-runner-v1/app-readers/README.md"),
    ),
    (
        "display-tool-times.json",
        include_str!("../test-vectors/llm-runner-v1/display-tool-times.json"),
    ),
    (
        "tool-upfront/valid.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/valid.json"),
    ),
    (
        "tool-upfront/undeclared.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/undeclared.json"),
    ),
    (
        "tool-upfront/empty.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/empty.json"),
    ),
    (
        "tool-upfront/duplicate.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/duplicate.json"),
    ),
    (
        "tool-upfront/non-string.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/non-string.json"),
    ),
    (
        "tool-upfront/non-array.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/non-array.json"),
    ),
    (
        "tool-upfront/stale.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/stale.json"),
    ),
    (
        "tool-upfront/without-upfront.json",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/without-upfront.json"),
    ),
    (
        "tool-upfront/README.md",
        include_str!("../test-vectors/llm-runner-v1/tool-upfront/README.md"),
    ),
    (
        "truncate.json",
        include_str!("../test-vectors/llm-runner-v1/truncate.json"),
    ),
    (
        "pi-1.3.0-1.4.0.jsonl",
        include_str!("../test-vectors/llm-runner-v1/pi-1.3.0-1.4.0.jsonl"),
    ),
    (
        "pi-session.json",
        include_str!("../test-vectors/llm-runner-v1/pi-session.json"),
    ),
    (
        "display-pi.json",
        include_str!("../test-vectors/llm-runner-v1/display-pi.json"),
    ),
    (
        "README-pi.md",
        include_str!("../test-vectors/llm-runner-v1/README-pi.md"),
    ),
    (
        "refresh-relaunch-required.json",
        include_str!("../test-vectors/llm-runner-v1/refresh-relaunch-required.json"),
    ),
    (
        "rich-ui-stamp.json",
        include_str!("../test-vectors/llm-runner-v1/rich-ui-stamp.json"),
    ),
    (
        "head-last-run-error.json",
        include_str!("../test-vectors/llm-runner-v1/head-last-run-error.json"),
    ),
    (
        "archive-prunes-reasoning.json",
        include_str!("../test-vectors/llm-runner-v1/archive-prunes-reasoning.json"),
    ),
    (
        "reasoning-pruned.json",
        include_str!("../test-vectors/llm-runner-v1/reasoning-pruned.json"),
    ),
    (
        "reasoning-pruned-profiles.json",
        include_str!("../test-vectors/llm-runner-v1/reasoning-pruned-profiles.json"),
    ),
    (
        "call-identity.json",
        include_str!("../test-vectors/llm-runner-v1/call-identity.json"),
    ),
    (
        "retention-floor.json",
        include_str!("../test-vectors/llm-runner-v1/retention-floor.json"),
    ),
    (
        "log-read-watermark.json",
        include_str!("../test-vectors/llm-runner-v1/log-read-watermark.json"),
    ),
    (
        "session-retention.json",
        include_str!("../test-vectors/llm-runner-v1/session-retention.json"),
    ),
    (
        "step-events.json",
        include_str!("../test-vectors/llm-runner-v1/step-events.json"),
    ),
    (
        "system-text-default.json",
        include_str!("../test-vectors/llm-runner-v1/system-text-default.json"),
    ),
    (
        "head-incarnation.json",
        include_str!("../test-vectors/llm-runner-v1/head-incarnation.json"),
    ),
    (
        "plan-compaction-without-preset.json",
        include_str!("../test-vectors/llm-runner-v1/plan-compaction-without-preset.json"),
    ),
    (
        "head-sender.json",
        include_str!("../test-vectors/llm-runner-v1/head-sender.json"),
    ),
    (
        "head-session-usage-bounds.json",
        include_str!("../test-vectors/llm-runner-v1/head-session-usage-bounds.json"),
    ),
    (
        "head-session-usage.json",
        include_str!("../test-vectors/llm-runner-v1/head-session-usage.json"),
    ),
    (
        "output-refs.json",
        include_str!("../test-vectors/llm-runner-v1/output-refs.json"),
    ),
    (
        "attachments.json",
        include_str!("../test-vectors/llm-runner-v1/attachments.json"),
    ),
    (
        "read-descriptors.json",
        include_str!("../test-vectors/llm-runner-v1/read-descriptors.json"),
    ),
    (
        "user-commands.json",
        include_str!("../test-vectors/llm-runner-v1/user-commands.json"),
    ),
    (
        "admit-optional-text-refused.json",
        include_str!("../test-vectors/llm-runner-v1/admit-optional-text-refused.json"),
    ),
    (
        "admit-optional-text-timeout.json",
        include_str!("../test-vectors/llm-runner-v1/admit-optional-text-timeout.json"),
    ),
    (
        "admit-optional-text-unknown-provider.json",
        include_str!("../test-vectors/llm-runner-v1/admit-optional-text-unknown-provider.json"),
    ),
    (
        "adopt-describe.json",
        include_str!("../test-vectors/llm-runner-v1/adopt-describe.json"),
    ),
    (
        "adopt.json",
        include_str!("../test-vectors/llm-runner-v1/adopt.json"),
    ),
    (
        "baseline.json",
        include_str!("../test-vectors/llm-runner-v1/baseline.json"),
    ),
    (
        "context-ceiling.json",
        include_str!("../test-vectors/llm-runner-v1/context-ceiling.json"),
    ),
    (
        "delivery-order-describe.json",
        include_str!("../test-vectors/llm-runner-v1/delivery-order-describe.json"),
    ),
    (
        "delivery-order-modes-describe.json",
        include_str!("../test-vectors/llm-runner-v1/delivery-order-modes-describe.json"),
    ),
    (
        "delivery-order.json",
        include_str!("../test-vectors/llm-runner-v1/delivery-order.json"),
    ),
    (
        "directives/200-byte-note.json",
        include_str!("../test-vectors/llm-runner-v1/directives/200-byte-note.json"),
    ),
    (
        "directives/bad-duration.json",
        include_str!("../test-vectors/llm-runner-v1/directives/bad-duration.json"),
    ),
    (
        "directives/bad-note.json",
        include_str!("../test-vectors/llm-runner-v1/directives/bad-note.json"),
    ),
    (
        "directives/empty-note.json",
        include_str!("../test-vectors/llm-runner-v1/directives/empty-note.json"),
    ),
    (
        "directives/empty.json",
        include_str!("../test-vectors/llm-runner-v1/directives/empty.json"),
    ),
    (
        "directives/fenced.json",
        include_str!("../test-vectors/llm-runner-v1/directives/fenced.json"),
    ),
    (
        "directives/in.json",
        include_str!("../test-vectors/llm-runner-v1/directives/in.json"),
    ),
    (
        "directives/malformed.json",
        include_str!("../test-vectors/llm-runner-v1/directives/malformed.json"),
    ),
    (
        "directives/max-hours.json",
        include_str!("../test-vectors/llm-runner-v1/directives/max-hours.json"),
    ),
    (
        "directives/mid-text.json",
        include_str!("../test-vectors/llm-runner-v1/directives/mid-text.json"),
    ),
    (
        "directives/note.json",
        include_str!("../test-vectors/llm-runner-v1/directives/note.json"),
    ),
    (
        "directives/off.json",
        include_str!("../test-vectors/llm-runner-v1/directives/off.json"),
    ),
    (
        "directives/parts.json",
        include_str!("../test-vectors/llm-runner-v1/directives/parts.json"),
    ),
    (
        "directives/too-long.json",
        include_str!("../test-vectors/llm-runner-v1/directives/too-long.json"),
    ),
    (
        "directives/unclosed-fence.json",
        include_str!("../test-vectors/llm-runner-v1/directives/unclosed-fence.json"),
    ),
    (
        "directives/whitespace.json",
        include_str!("../test-vectors/llm-runner-v1/directives/whitespace.json"),
    ),
    (
        "directives/wire.json",
        include_str!("../test-vectors/llm-runner-v1/directives/wire.json"),
    ),
    (
        "errors.json",
        include_str!("../test-vectors/llm-runner-v1/errors.json"),
    ),
    (
        "head.json",
        include_str!("../test-vectors/llm-runner-v1/head.json"),
    ),
    (
        "history.json",
        include_str!("../test-vectors/llm-runner-v1/history.json"),
    ),
    (
        "input-token-cap-describe.json",
        include_str!("../test-vectors/llm-runner-v1/input-token-cap-describe.json"),
    ),
    (
        "input-token-cap-results.json",
        include_str!("../test-vectors/llm-runner-v1/input-token-cap-results.json"),
    ),
    (
        "input-token-cap.json",
        include_str!("../test-vectors/llm-runner-v1/input-token-cap.json"),
    ),
    (
        "interrupt-reasons.json",
        include_str!("../test-vectors/llm-runner-v1/interrupt-reasons.json"),
    ),
    (
        "plans.json",
        include_str!("../test-vectors/llm-runner-v1/plans.json"),
    ),
    (
        "read-pages.json",
        include_str!("../test-vectors/llm-runner-v1/read-pages.json"),
    ),
    (
        "read-requests.json",
        include_str!("../test-vectors/llm-runner-v1/read-requests.json"),
    ),
    (
        "refuse-later-plan-after-planless-first-episode.json",
        include_str!(
            "../test-vectors/llm-runner-v1/refuse-later-plan-after-planless-first-episode.json"
        ),
    ),
    (
        "refuse-later-send-with-different-plan.json",
        include_str!("../test-vectors/llm-runner-v1/refuse-later-send-with-different-plan.json"),
    ),
    (
        "refuse-tool-name-collision.json",
        include_str!("../test-vectors/llm-runner-v1/refuse-tool-name-collision.json"),
    ),
    (
        "reply-envelope.json",
        include_str!("../test-vectors/llm-runner-v1/reply-envelope.json"),
    ),
    (
        "reply-too-large.json",
        include_str!("../test-vectors/llm-runner-v1/reply-too-large.json"),
    ),
    (
        "role-describe.json",
        include_str!("../test-vectors/llm-runner-v1/role-describe.json"),
    ),
    (
        "run-cancel-describe.json",
        include_str!("../test-vectors/llm-runner-v1/run-cancel-describe.json"),
    ),
    (
        "run-cancel.json",
        include_str!("../test-vectors/llm-runner-v1/run-cancel.json"),
    ),
    (
        "run-parameters-describe.json",
        include_str!("../test-vectors/llm-runner-v1/run-parameters-describe.json"),
    ),
    (
        "run-parameters.json",
        include_str!("../test-vectors/llm-runner-v1/run-parameters.json"),
    ),
    (
        "run-result.json",
        include_str!("../test-vectors/llm-runner-v1/run-result.json"),
    ),
    (
        "run-tool-calls-describe.json",
        include_str!("../test-vectors/llm-runner-v1/run-tool-calls-describe.json"),
    ),
    (
        "run-tool-calls.json",
        include_str!("../test-vectors/llm-runner-v1/run-tool-calls.json"),
    ),
    (
        "run_status.usage_disjoint_input.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_disjoint_input.json"),
    ),
    (
        "run_status.usage_model_switch.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_model_switch.json"),
    ),
    (
        "run_status.usage_plain.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_plain.json"),
    ),
    (
        "run_status.usage_reasoning.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_reasoning.json"),
    ),
    (
        "run_status.usage_unreported_step.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_unreported_step.json"),
    ),
    (
        "send-status-describe.json",
        include_str!("../test-vectors/llm-runner-v1/send-status-describe.json"),
    ),
    (
        "send-status.json",
        include_str!("../test-vectors/llm-runner-v1/send-status.json"),
    ),
    (
        "send.json",
        include_str!("../test-vectors/llm-runner-v1/send.json"),
    ),
    (
        "serializer-anthropic-messages.json",
        include_str!("../test-vectors/llm-runner-v1/serializer-anthropic-messages.json"),
    ),
    (
        "session-change.json",
        include_str!("../test-vectors/llm-runner-v1/session-change.json"),
    ),
    (
        "session-ref-v1.json",
        include_str!("../test-vectors/llm-runner-v1/session-ref-v1.json"),
    ),
    (
        "session-ref.json",
        include_str!("../test-vectors/llm-runner-v1/session-ref.json"),
    ),
    (
        "structured-output-describe.json",
        include_str!("../test-vectors/llm-runner-v1/structured-output-describe.json"),
    ),
    (
        "structured-output-results.json",
        include_str!("../test-vectors/llm-runner-v1/structured-output-results.json"),
    ),
    (
        "structured-output.json",
        include_str!("../test-vectors/llm-runner-v1/structured-output.json"),
    ),
    (
        "subscribe.json",
        include_str!("../test-vectors/llm-runner-v1/subscribe.json"),
    ),
    (
        "tag-checkpoint-describe.json",
        include_str!("../test-vectors/llm-runner-v1/tag-checkpoint-describe.json"),
    ),
    (
        "message-checkpoint.json",
        include_str!("../test-vectors/llm-runner-v1/message-checkpoint.json"),
    ),
    (
        "tag-cleaning.json",
        include_str!("../test-vectors/llm-runner-v1/tag-cleaning.json"),
    ),
    (
        "tag-numbering.json",
        include_str!("../test-vectors/llm-runner-v1/tag-numbering.json"),
    ),
    (
        "tags-describe.json",
        include_str!("../test-vectors/llm-runner-v1/tags-describe.json"),
    ),
    (
        "tags.json",
        include_str!("../test-vectors/llm-runner-v1/tags.json"),
    ),
    (
        "display-anthropic-messages.json",
        include_str!("../test-vectors/llm-runner-v1/display-anthropic-messages.json"),
    ),
    (
        "display-opencode-aisdk.json",
        include_str!("../test-vectors/llm-runner-v1/display-opencode-aisdk.json"),
    ),
    (
        "display-requests.json",
        include_str!("../test-vectors/llm-runner-v1/display-requests.json"),
    ),
    (
        "live-events.json",
        include_str!("../test-vectors/llm-runner-v1/live-events.json"),
    ),
    (
        "run_status.held.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.held.json"),
    ),
    (
        "run_status.held_after_late.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.held_after_late.json"),
    ),
    (
        "run_status.held_absent.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.held_absent.json"),
    ),
    (
        "turn-system-rendering.json",
        include_str!("../test-vectors/llm-runner-v1/turn-system-rendering.json"),
    ),
    (
        "README.md",
        include_str!("../test-vectors/llm-runner-v1/README.md"),
    ),
    (
        "opencode-v2-rows-2.0.26.json",
        include_str!("../test-vectors/llm-runner-v1/opencode-v2-rows-2.0.26.json"),
    ),
    (
        "opencode-v2-rows-2.2.3.json",
        include_str!("../test-vectors/llm-runner-v1/opencode-v2-rows-2.2.3.json"),
    ),
    (
        "opencode-v2-session.json",
        include_str!("../test-vectors/llm-runner-v1/opencode-v2-session.json"),
    ),
    (
        "display-opencode-v2.json",
        include_str!("../test-vectors/llm-runner-v1/display-opencode-v2.json"),
    ),
    (
        "extract-opencode-v2.py",
        include_str!("../test-vectors/llm-runner-v1/extract-opencode-v2.py"),
    ),
    (
        "README-opencode-v2.md",
        include_str!("../test-vectors/llm-runner-v1/README-opencode-v2.md"),
    ),
    (
        "opencode-v2-reinjection-rows-2.0.26.json",
        include_str!("../test-vectors/llm-runner-v1/opencode-v2-reinjection-rows-2.0.26.json"),
    ),
    (
        "opencode-v2-reinjection-rows-2.2.3.json",
        include_str!("../test-vectors/llm-runner-v1/opencode-v2-reinjection-rows-2.2.3.json"),
    ),
    (
        "served-bytes.json",
        include_str!("../test-vectors/llm-runner-v1/served-bytes.json"),
    ),
];

/// `sha256sum` output over [`FILES`]: one `<hex>  <name>` line per file.
pub const SHA256SUMS: &str = include_str!("../test-vectors/llm-runner-v1/SHA256SUMS");

/// The vectors' `README.md`.
pub const README: &str = include_str!("../test-vectors/llm-runner-v1/README.md");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}

/// Test helpers over the vectors, for this crate's own tests.
#[cfg(test)]
pub(crate) use test_support::*;

#[cfg(test)]
mod test_support {
    use serde::{de::DeserializeOwned, Serialize};
    use serde_json::Value;

    use super::file;

    /// The vector file `name`, decoded.
    pub fn load(name: &str) -> Value {
        let text = file(name).unwrap_or_else(|| panic!("{name} is not listed in FILES"));
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// Re-encode a typed object inside an unchanged upstream fixture envelope.
    /// Compare actual bytes, including field order and indentation, rather than
    /// normalizing through `Value`, which would hide order and whitespace changes.
    pub fn exact_object_round_trip<T>(name: &str, marker: &str, indent: usize) -> T
    where
        T: DeserializeOwned + Serialize,
    {
        let bytes = file(name)
            .unwrap_or_else(|| panic!("{name} is not listed in FILES"))
            .to_owned();
        let start = bytes.find(marker).unwrap() + marker.len();
        let mut stream = serde_json::Deserializer::from_str(&bytes[start..]).into_iter::<Value>();
        stream.next().unwrap().unwrap();
        let end = start + stream.byte_offset();
        let decoded: T = serde_json::from_str(&bytes[start..end]).unwrap();
        let encoded = serde_json::to_string_pretty(&decoded)
            .unwrap()
            .replace('\n', &format!("\n{}", " ".repeat(indent)));
        let mut rebuilt = bytes.clone();
        rebuilt.replace_range(start..end, &encoded);
        assert_eq!(rebuilt.as_bytes(), bytes.as_bytes(), "{name}: bytes differ");
        decoded
    }

    /// The array at `key`, panicking with the key's name if it is missing or
    /// not an array.
    pub fn cases<'a>(file: &'a Value, key: &str) -> &'a Vec<Value> {
        file[key]
            .as_array()
            .unwrap_or_else(|| panic!("vector key {key} is not an array"))
    }

    /// Decode `json` as `T`, then check that encoding it gives back exactly
    /// `json` and that decoding the encoding gives the same value. Canonical
    /// vectors carry no unknown fields, so a field the type drops or renames
    /// shows up here.
    pub fn round_trip<T>(name: &str, json: &Value) -> T
    where
        T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug,
    {
        let decoded: T = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{name}: does not decode: {e}"));
        let encoded = serde_json::to_value(&decoded).unwrap();
        assert_eq!(&encoded, json, "{name}: encoding differs from the vector");
        let again: T = serde_json::from_value(encoded).unwrap();
        assert_eq!(again, decoded, "{name}: second decode differs");
        decoded
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use sha2::{Digest, Sha256};

    use super::*;

    fn dir() -> String {
        format!("{}/test-vectors/{DIR}", env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn turn_system_rendering_vectors_preserve_order_placement_and_authority() {
        use serde_json::Value;

        fn legal_anthropic_boundary(before: &[Value], after: &[Value]) -> bool {
            let mut pending_tools = BTreeSet::new();
            for message in before {
                if let Some(blocks) = message["content"].as_array() {
                    for block in blocks {
                        match block["type"].as_str().unwrap() {
                            "tool_use" | "server_tool_use" => {
                                pending_tools.insert(block["id"].as_str().unwrap());
                            }
                            kind if kind == "tool_result" || kind.ends_with("_tool_result") => {
                                pending_tools.remove(block["tool_use_id"].as_str().unwrap());
                            }
                            _ => {}
                        }
                    }
                }
            }
            let follows_user_or_server_result = before.last().is_some_and(|message| {
                message["role"] == "user"
                    || (message["role"] == "assistant"
                        && message["content"]
                            .as_array()
                            .and_then(|blocks| blocks.last())
                            .and_then(|block| block["type"].as_str())
                            .is_some_and(|kind| kind.ends_with("_tool_result")))
            });
            pending_tools.is_empty()
                && follows_user_or_server_result
                && after
                    .first()
                    .is_none_or(|message| message["role"] == "assistant")
        }

        let vectors = load("turn-system-rendering.json");
        let cases = cases(&vectors, "cases");
        assert_eq!(cases.len(), 6);
        let mut ids = BTreeSet::new();
        let mut held = 0;
        for case in cases {
            let id = case["id"].as_str().unwrap();
            assert!(ids.insert(id), "duplicate case: {id}");
            let protocol = case["route"]["protocol"].as_str().unwrap();
            let supported = case["route"]["turn_system_supported"].as_bool().unwrap();
            let text = case["rendered_text"].as_str().unwrap();
            let attempts = case["attempts"].as_array().unwrap();
            assert!(!attempts.is_empty(), "{id}");
            for attempt in attempts {
                let conversation = attempt["conversation"].as_array().unwrap();
                let index = attempt["insertion_index"].as_u64().unwrap() as usize;
                assert!(index <= conversation.len(), "{id}");
                let expected = &attempt["expect"];
                if expected["outcome"] == "delivered" {
                    assert_eq!(index, conversation.len(), "{id}: delivery must append");
                }
                if expected["outcome"] == "not_possible_at_this_boundary" {
                    held += 1;
                    assert_eq!(protocol, "anthropic_messages", "{id}");
                    assert!(supported, "{id}");
                    assert!(
                        !legal_anthropic_boundary(&conversation[..index], &conversation[index..]),
                        "{id}: the held boundary must be illegal"
                    );
                    assert_eq!(expected["send_state"], "pending", "{id}");
                    assert!(expected["request"].is_null(), "{id}");
                    continue;
                }

                let request = &expected["request"];
                let key = match protocol {
                    "openai_responses" => "input",
                    "anthropic_messages" => "messages",
                    "gemini" => "contents",
                    _ => panic!("{id}: unknown protocol"),
                };
                let messages = request[key].as_array().unwrap();
                assert_eq!(messages.len(), conversation.len() + 1, "{id}");
                let mut without_entry = messages.clone();
                let entry = without_entry.remove(index);
                assert_eq!(&without_entry, conversation, "{id}: conversation order");
                assert!(request.get("instructions").is_none(), "{id}");
                if supported {
                    assert_eq!(expected["outcome"], "delivered", "{id}");
                    assert_eq!(expected["authority"], "system", "{id}");
                    assert_eq!(expected["delivered_as"]["mode"], "turn_system", "{id}");
                    if protocol == "openai_responses" {
                        assert_eq!(entry["role"], "developer", "{id}");
                        assert_eq!(entry["content"][0]["type"], "input_text", "{id}");
                        assert_eq!(entry["content"][0]["text"], text, "{id}");
                    } else {
                        assert_eq!(protocol, "anthropic_messages", "{id}");
                        assert_eq!(entry["role"], "system", "{id}");
                        assert_eq!(entry["content"], text, "{id}");
                        for (position, message) in messages.iter().enumerate() {
                            if message["role"] == "system" {
                                assert!(
                                    legal_anthropic_boundary(
                                        &messages[..position],
                                        &messages[position + 1..]
                                    ),
                                    "{id}: illegal native system position"
                                );
                            }
                        }
                    }
                } else {
                    assert_eq!(expected["authority"], "user", "{id}");
                    assert_eq!(entry["role"], "user", "{id}");
                    assert!(
                        messages.iter().all(|message| {
                            message["role"] != "system" && message["role"] != "developer"
                        }),
                        "{id}: unsupported route must not elevate authority"
                    );
                    let rendered = if protocol == "gemini" {
                        entry["parts"][0]["text"].as_str().unwrap()
                    } else {
                        entry["content"].as_str().unwrap()
                    };
                    assert_eq!(
                        rendered,
                        format!("<context-note>{text}</context-note>"),
                        "{id}"
                    );
                    if let Some(stored) = case.get("stored_entry") {
                        assert_eq!(expected["outcome"], "replayed", "{id}");
                        assert_eq!(stored["role"], "system", "{id}");
                        assert_eq!(stored["text"], text, "{id}");
                        assert_eq!(expected["delivered_as"], stored["delivered_as"], "{id}");
                        assert_eq!(expected["stored_entry_unchanged"], true, "{id}");
                    } else {
                        assert_eq!(expected["outcome"], "delivered", "{id}");
                        assert_eq!(expected["delivered_as"]["mode"], "steer", "{id}");
                    }
                }
            }
        }
        assert_eq!(held, 2);
    }

    /// The compiled-in bytes are exactly the files on disk, and every file
    /// in the directory is listed, so a new file cannot ship unread.
    #[test]
    fn every_vector_file_on_disk_is_listed_with_its_bytes() {
        let dir = dir();
        fn collect(dir: &std::path::Path, prefix: &str, names: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let name = format!("{prefix}{}", entry.file_name().to_str().unwrap());
                if entry.file_type().unwrap().is_dir() {
                    collect(&entry.path(), &format!("{name}/"), names);
                } else if entry.file_name() != "SHA256SUMS" {
                    names.push(name);
                }
            }
        }
        let mut on_disk = Vec::new();
        collect(std::path::Path::new(&dir), "", &mut on_disk);
        on_disk.sort();
        let mut listed: Vec<String> = FILES.iter().map(|(name, _)| name.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
        for (name, contents) in FILES {
            let bytes = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
            assert_eq!(*contents, bytes, "{name}");
            assert_eq!(file(name), Some(*contents));
        }
        assert_eq!(file("no-such-file.json"), None);
        assert_eq!(Some(README), file("README.md"));
        let sums = std::fs::read_to_string(format!("{dir}/SHA256SUMS")).unwrap();
        assert_eq!(SHA256SUMS, sums);
    }

    /// `SHA256SUMS` names every file once, in `FILES` order, with the
    /// SHA-256 of its bytes.
    #[test]
    fn files_match_sha256sums() {
        let names: Vec<&str> = SHA256SUMS
            .lines()
            .map(|line| line.split_once("  ").expect("sha256sum line").1)
            .collect();
        let in_order: Vec<&str> = FILES.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, in_order, "SHA256SUMS lists FILES in order");
        let checksums: BTreeMap<&str, &str> = SHA256SUMS
            .lines()
            .map(|line| {
                let (digest, name) = line.split_once("  ").unwrap();
                (name, digest)
            })
            .collect();
        assert_eq!(
            checksums.keys().copied().collect::<BTreeSet<_>>(),
            in_order.iter().copied().collect::<BTreeSet<_>>()
        );
        for (name, contents) in FILES {
            assert_eq!(
                format!("{:x}", Sha256::digest(contents.as_bytes())),
                checksums[name],
                "{name}: SHA-256 mismatch"
            );
        }
    }
}
