//! `transform.hook`: one hook call.
//!
//! Every call carries the session's opaque name, the plan item's preset and
//! params (so the provider resolves the same frozen config on every call
//! without remembering it), and the subject: the value the hook may change,
//! with what identifies it. The runner calls a hook before the record it
//! transforms is written, so a subject has no ordinal yet.
//!
//! Decoded leniently on fields, strictly on `hook` and `phase`: a provider
//! that does not know a hook refuses the call rather than answer it as
//! another.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::subscription::{Hook, Phase};

/// The `transform.hook` request.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct HookCall {
    /// The runner's opaque name for the session. A provider may key its own
    /// per-session state on it, and never uses it to choose what it returns.
    pub session: String,
    /// The session's current lineage. Absent only on `pre_user` for the
    /// first message of a session nothing has been written to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lineage_id: Option<String>,
    /// The plan item's preset, verbatim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    /// The plan item's params, verbatim.
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(flatten)]
    pub subject: Subject,
}

/// What the hook acts on, tagged by `hook`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "hook", rename_all = "snake_case")]
pub enum Subject {
    /// A user message, or a steered or queued prompt, before it is written.
    PreUser {
        /// The message's text.
        text: String,
        /// The sender's mark on a steered or queued prompt, opaque, as the
        /// owner sent it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mark: Option<Value>,
        /// `queue`, `steer` or `interrupt` for an owner's prompt; absent for
        /// a human turn. Decoded open.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        delivery: Option<String>,
    },
    /// A completed assistant message's text, before it is written. Never
    /// reasoning, signatures or tool calls.
    PostAssistant {
        /// The step that produced the message, opaque.
        step_id: String,
        text: String,
    },
    /// A tool call before it executes.
    PreTool {
        step_id: String,
        phase: Phase,
        /// The tool's model-facing name.
        tool: String,
        /// The model's id for the call: display only, not unique.
        tool_call_id: String,
        /// The runner's key for the call, when one is minted already (a
        /// deferred call's prepared record). Usually absent: the key is
        /// minted from the dispatch intent, written after this hook.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_key: Option<String>,
        /// The input that would execute: the model's input after every
        /// earlier `mutate`.
        input: Value,
    },
    /// A tool's result before it is written.
    PostTool {
        step_id: String,
        tool: String,
        tool_call_id: String,
        /// The runner's key for the call.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        call_key: Option<String>,
        /// The result's text.
        text: String,
        is_error: bool,
    },
}

impl Subject {
    pub fn hook(&self) -> Hook {
        match self {
            Self::PreUser { .. } => Hook::PreUser,
            Self::PostAssistant { .. } => Hook::PostAssistant,
            Self::PreTool { .. } => Hook::PreTool,
            Self::PostTool { .. } => Hook::PostTool,
        }
    }

    pub fn phase(&self) -> Option<Phase> {
        match self {
            Self::PreTool { phase, .. } => Some(*phase),
            _ => None,
        }
    }

    /// The tool the subject concerns, on the tool hooks.
    pub fn tool(&self) -> Option<&str> {
        match self {
            Self::PreTool { tool, .. } | Self::PostTool { tool, .. } => Some(tool),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn hook_request_vectors_round_trip() {
        let file = vectors::load("hook-requests.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let call = vectors::round_trip::<HookCall>(name, &case["request"]);
            let hook: Hook = serde_json::from_value(case["request"]["hook"].clone()).unwrap();
            assert_eq!(call.subject.hook(), hook, "{name}");
            assert_eq!(
                call.subject
                    .phase()
                    .map(|p| serde_json::to_value(p).unwrap()),
                case["request"].get("phase").cloned(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<HookCall>(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<HookCall>(case["name"].as_str().unwrap(), &case["request"]);
        }
    }
}
