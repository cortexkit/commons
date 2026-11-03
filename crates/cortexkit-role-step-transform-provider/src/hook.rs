//! Hook requests carry subjects, never caller identity. The route supplies
//! the session binding, attested runner and project scope.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::subscription::{Hook, Phase, SubjectPartProblem};

/// Optional compact-JSON request cap for hooks carrying a whole message.
/// The 3 MiB cap leaves room below the daemon's 4 MiB connection queue.
pub const DEFAULT_HOOK_CAP_BYTES: usize = 3 * 1024 * 1024;

/// The `transform.hook` request, on a session-bound route.
///
/// For every hook request, call [`Self::check_subject_fields`] to reject
/// unknown hooks/phases, ordinal zero and invalid subject/message pairing,
/// and call [`Self::validate`] to check `subject_part`. Deserialization alone
/// does not perform these semantic checks. Before interpreting a supplied
/// `message`, additionally call [`Self::check_message_profile`]. The
/// whole-message size check ([`Self::check_message_size`]) is optional.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct HookCall {
    /// Stable message id, independent of the entry ordinal. Present together
    /// with `subject_ordinal` or not at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_mid: Option<String>,
    /// Entry position counted from 1, following compaction W4.1/W4.2.
    /// Present together with `subject_mid` or not at all; zero is invalid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_ordinal: Option<u64>,
    /// Whole message in the schema named by `params.serializer_profile`.
    /// Explicit null is present and requires both subject identity fields.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_message"
    )]
    pub message: Option<Value>,
    /// Opaque part identity, not the display-only tool call id. When present,
    /// must be nonempty and at most 256 UTF-8 bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_part: Option<String>,
    /// Optional runner diagnostics, carried without interpretation on any hook.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present_message"
    )]
    pub runner_trace: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default)]
    pub params: Map<String, Value>,
    #[serde(flatten)]
    pub subject: Subject,
}

impl HookCall {
    pub fn new(subject: Subject) -> Self {
        Self {
            subject_mid: None,
            subject_ordinal: None,
            message: None,
            subject_part: None,
            runner_trace: None,
            preset: None,
            params: Map::new(),
            subject,
        }
    }

    pub fn with_subject_identity(mut self, mid: impl Into<String>, ordinal: u64) -> Self {
        self.subject_mid = Some(mid.into());
        self.subject_ordinal = Some(ordinal);
        self
    }

    pub fn with_message(mut self, message: Value) -> Self {
        self.message = Some(message);
        self
    }

    pub fn with_subject_part(mut self, subject_part: impl Into<String>) -> Self {
        self.subject_part = Some(subject_part.into());
        self
    }

    pub fn with_runner_trace(mut self, trace: Value) -> Self {
        self.runner_trace = Some(trace);
        self
    }

    /// Check only the nonempty, at-most-256-UTF-8-byte `subject_part` rule,
    /// without normalization. Also call [`Self::check_subject_fields`] on
    /// every hook request, even when no whole message is supplied.
    pub fn validate(&self) -> Result<(), SubjectPartProblem> {
        crate::subscription::check_subject_part(self.subject_part.as_deref())
    }

    /// On every hook request, reject unknown hooks/phases, ordinal zero and
    /// invalid subject/message pairing, even when `message` is absent.
    /// Deserialization alone does not perform these semantic checks.
    /// Refuse a failure with `invalid_params`, naming the problem's field.
    pub fn check_subject_fields(&self) -> Result<(), HookProblem> {
        if self.subject_ordinal == Some(0) {
            return Err(HookProblem::SubjectOrdinalZero);
        }
        if matches!(self.subject, Subject::Unknown { .. })
            || matches!(self.subject.phase(), Some(Phase::Unknown(_)))
        {
            return Err(HookProblem::UnknownSubject);
        }
        if self.message.is_some() && (self.subject_mid.is_none() || self.subject_ordinal.is_none())
        {
            return Err(HookProblem::MessageWithoutSubject);
        }
        match (&self.subject_mid, self.subject_ordinal) {
            (Some(_), None) => Err(HookProblem::SubjectOrdinalMissing),
            (None, Some(_)) => Err(HookProblem::SubjectMidMissing),
            _ => Ok(()),
        }
    }

    /// Optionally enforce a request cap when a whole message is supplied.
    /// All request fields count, not just `message`. Calls without `message`
    /// retain their existing size policy. Refuse with `invalid_params`, never
    /// silently truncate. [`DEFAULT_HOOK_CAP_BYTES`] is the default cap.
    pub fn check_message_size(&self, cap_bytes: usize) -> Result<(), HookProblem> {
        if self.message.is_some()
            && serde_json::to_vec(self)
                .expect("hook fields are JSON-serializable")
                .len()
                > cap_bytes
        {
            return Err(HookProblem::RequestTooLarge);
        }
        Ok(())
    }

    /// Call before interpreting `message`; a provider may instead ignore it
    /// when the required history is available through another supported path.
    pub fn check_message_profile(&self, supported: &[&str]) -> Result<(), HookProblem> {
        if self.message.is_some()
            && !self
                .params
                .get("serializer_profile")
                .and_then(Value::as_str)
                .is_some_and(|profile| supported.contains(&profile))
        {
            return Err(HookProblem::SerializerProfile);
        }
        Ok(())
    }

    pub fn with_item(mut self, preset: Option<String>, params: Map<String, Value>) -> Self {
        self.preset = preset;
        self.params = params;
        self
    }
}

// Explicit JSON null is a present opaque message, not an absent field. It
// must still satisfy pairing and survive a round trip without interpretation.
fn present_message<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// Why subject fields are invalid. All use `invalid_params`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HookProblem {
    MessageWithoutSubject,
    SubjectOrdinalMissing,
    SubjectOrdinalZero,
    UnknownSubject,
    SerializerProfile,
    SubjectMidMissing,
    RequestTooLarge,
}

impl HookProblem {
    /// The `detail.field` a provider names in its refusal.
    pub fn field(self) -> &'static str {
        match self {
            Self::MessageWithoutSubject | Self::RequestTooLarge => "message",
            Self::SubjectOrdinalMissing | Self::SubjectOrdinalZero => "subject_ordinal",
            Self::UnknownSubject => "hook",
            Self::SerializerProfile => "params.serializer_profile",
            Self::SubjectMidMissing => "subject_mid",
        }
    }
}

open_tagged! {
/// What the hook acts on, tagged by `hook`.
#[derive(Clone, Debug, PartialEq)]
pub enum Subject, "hook" {
    /// A user message, or a steered or queued prompt, before it is written.
    PreUser => "pre_user" {
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
        /// The message's time in milliseconds since the Unix epoch (UTC), from
        /// the runner's machine: the host's creation time when the host stamps
        /// it, otherwise the runner's clock at the hook call. With it, a
        /// provider measures the gap since the previous message on one
        /// machine's clock. When it's absent, the provider uses its own clock.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at_ms: Option<u64>,
    },
    /// A completed assistant message's text, before it is written. Never
    /// reasoning, signatures or tool calls.
    PostAssistant => "post_assistant" {
        /// The step that produced the message, opaque.
        step_id: String,
        /// The text of each of the message's text blocks, in message order.
        /// Reasoning, signatures and tool calls are not listed.
        blocks: Vec<String>,
    },
    /// A tool call before it executes.
    PreTool => "pre_tool" {
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
    PostTool => "post_tool" {
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
}

impl Subject {
    pub fn hook(&self) -> Hook {
        match self {
            Self::PreUser { .. } => Hook::PreUser,
            Self::PostAssistant { .. } => Hook::PostAssistant,
            Self::PreTool { .. } => Hook::PreTool,
            Self::PostTool { .. } => Hook::PostTool,
            Self::Unknown { name, .. } => Hook::Unknown(name.clone()),
        }
    }

    pub fn phase(&self) -> Option<Phase> {
        match self {
            Self::PreTool { phase, .. } => Some(phase.clone()),
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
            Self::PreTool { .. } | Self::Unknown { .. } => None,
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
    fn every_hook_uses_route_identity_only() {
        let file = vectors::load("hook-requests.json");
        let mut hooks = Vec::new();
        for case in vectors::cases(&file, "requests") {
            let call =
                vectors::round_trip::<HookCall>(case["name"].as_str().unwrap(), &case["request"]);
            let wire = serde_json::to_value(&call).unwrap();
            for field in ["session", "harness", "lineage_id"] {
                assert!(wire.get(field).is_none(), "{field}");
            }
            hooks.push(call.subject.hook());
        }
        for hook in Hook::ALL {
            assert!(hooks.contains(&hook), "{hook:?} has no request vector");
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
        for case in vectors::cases(&file, "unsupported") {
            let name = case["name"].as_str().unwrap();
            let call = vectors::round_trip::<HookCall>(name, &case["request"]);
            assert_eq!(
                call.check_subject_fields(),
                Err(HookProblem::UnknownSubject),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "undecodable") {
            vectors::refused::<HookCall>(case["name"].as_str().unwrap(), &case["request"]);
        }
    }
}
