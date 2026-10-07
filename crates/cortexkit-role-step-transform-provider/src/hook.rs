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
#[non_exhaustive]
pub struct HookCall {
    /// The runner's opaque name for the session. A provider may key its own
    /// per-session state on it, and never uses it to choose what it returns.
    pub session: String,
    /// The harness named in the session's key, which identifies the caller
    /// (for example `broca`). This differs from the route's bind harness,
    /// the harness a connection declares when it opens the route; a runner
    /// always declares `runner`. Required: a request without it does
    /// not decode. A provider keys a runner conversation on
    /// `(project_root, session, harness)`.
    pub harness: String,
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

impl HookCall {
    pub fn new(session: impl Into<String>, harness: impl Into<String>, subject: Subject) -> Self {
        Self {
            session: session.into(),
            harness: harness.into(),
            lineage_id: None,
            preset: None,
            params: Map::new(),
            subject,
        }
    }

    pub fn with_lineage(mut self, lineage_id: impl Into<String>) -> Self {
        self.lineage_id = Some(lineage_id.into());
        self
    }

    pub fn with_item(mut self, preset: Option<String>, params: Map<String, Value>) -> Self {
        self.preset = preset;
        self.params = params;
        self
    }
}

/// What the hook acts on, tagged by `hook`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "hook", rename_all = "snake_case")]
pub enum Subject {
    /// A user message, or a steered or queued prompt, before it is written.
    PreUser {
        /// The text of each of the message's text blocks, in message order
        /// ([`Subject::blocks`]).
        blocks: Vec<String>,
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
        /// The text of each of the message's text blocks, in message order.
        /// Reasoning, signatures and tool calls are not listed.
        blocks: Vec<String>,
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
        /// The text of each of the result's text parts, in order; a result
        /// that is a single string is one block.
        blocks: Vec<String>,
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

    /// The subject's text blocks, which operations address by index:
    /// only the text blocks of the message or result, in order. Non-text
    /// blocks (images, tool calls, signed thinking) are neither listed nor
    /// addressable. `None` on `pre_tool`, which has no text subject.
    pub fn blocks(&self) -> Option<&[String]> {
        match self {
            Self::PreUser { blocks, .. }
            | Self::PostAssistant { blocks, .. }
            | Self::PostTool { blocks, .. } => Some(blocks),
            Self::PreTool { .. } => None,
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

    /// Every request in the `requests` list of `hook-requests.json`, which
    /// covers each hook at least once, names its caller's harness, and the
    /// field survives a round trip.
    #[test]
    fn every_hook_request_round_trips_its_harness() {
        let file = vectors::load("hook-requests.json");
        let mut hooks = Vec::new();
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let call = vectors::round_trip::<HookCall>(name, &case["request"]);
            assert_eq!(call.harness, "broca", "{name}");
            hooks.push(call.subject.hook());
        }
        for hook in Hook::ALL {
            assert!(hooks.contains(&hook), "{hook:?} has no request vector");
        }
    }

    /// `harness` is required: removing it from any request in the
    /// `requests` list of `hook-requests.json` makes the request fail to
    /// decode, and the error names the field.
    #[test]
    fn a_hook_request_without_harness_is_refused_by_name() {
        let file = vectors::load("hook-requests.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let mut request = case["request"].clone();
            assert!(
                request.as_object_mut().unwrap().remove("harness").is_some(),
                "{name}"
            );
            let error = serde_json::from_value::<HookCall>(request)
                .expect_err(name)
                .to_string();
            assert!(error.contains("missing field `harness`"), "{name}: {error}");
        }
    }

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
