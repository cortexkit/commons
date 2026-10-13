//! `agent_data.prepare {refs, cutId}`: on the source, decide where the
//! data ends and freeze it where possible.
//!
//! Each item feeds `cut.seal` unchanged (`sessionRef`, `stream`,
//! `endOffset`, `digest`). `format` names the data's store version for the
//! receiver's `import` to check; `frozen: false` means the participant
//! can't stop writes. `external` lists data the host program ships itself.
//! Before replying the participant checks with `session.cover` that
//! everything it admitted is covered, and makes the freeze durable.
//! Step: `none` → `prepared`.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `prepare` request: `{refs, cutId}`.
pub type PrepareRequest = crate::request::CutRequest;

/// The `prepare` reply: `{items, external}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PrepareReply {
    pub items: Vec<PreparedItem>,
    pub external: Vec<ExternalItem>,
}

impl PrepareReply {
    pub fn new(items: Vec<PreparedItem>, external: Vec<ExternalItem>) -> Self {
        Self { items, external }
    }
}

/// One stream of the cut, for one ref. Each ref yields at most one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PreparedItem {
    pub ref_kind: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// Exactly `cortexkit_role_llm_runner::session_ref` for the session
    /// this ref's data belongs to: 32 lowercase hex characters.
    #[serde(rename = "sessionRef")]
    pub session_ref: String,
    /// This participant's principal, `reserved:<module_id>`.
    pub stream: String,
    /// A length in bytes of that stream: the cut ends here.
    #[serde(rename = "endOffset")]
    pub end_offset: u64,
    /// SHA-256 over the stream's bytes `[0, endOffset)`, lowercase hex: the
    /// same digest engram's session door computes over those bytes.
    pub digest: String,
    /// Opaque text naming the data's store version. The receiver's `import`
    /// checks it before reading anything.
    pub format: String,
    /// False when this source participant can't stop writes: anything
    /// written after `endOffset` stays on the source Mac and becomes a fork
    /// if the agent is used there again.
    pub frozen: bool,
}

impl PreparedItem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ref_kind: impl Into<String>,
        r#ref: impl Into<String>,
        session_ref: impl Into<String>,
        stream: impl Into<String>,
        end_offset: u64,
        digest: impl Into<String>,
        format: impl Into<String>,
        frozen: bool,
    ) -> Self {
        Self {
            ref_kind: ref_kind.into(),
            r#ref: r#ref.into(),
            session_ref: session_ref.into(),
            stream: stream.into(),
            end_offset,
            digest: digest.into(),
            format: format.into(),
            frozen,
        }
    }
}

/// External data: a file the host program copies to the receiver itself,
/// so it never goes through the door.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ExternalItem {
    pub ref_kind: String,
    #[serde(rename = "ref")]
    pub r#ref: String,
    /// Operator text, for example "Claude Code session transcript: copy it
    /// with the project".
    pub note: String,
    /// Opaque to entorhinal, which passes it through unexamined: whatever
    /// the receiving participant needs to recognise the right host data.
    /// Required; any JSON value is accepted.
    pub check: Value,
}

impl ExternalItem {
    pub fn new(
        ref_kind: impl Into<String>,
        r#ref: impl Into<String>,
        note: impl Into<String>,
        check: Value,
    ) -> Self {
        Self {
            ref_kind: ref_kind.into(),
            r#ref: r#ref.into(),
            note: note.into(),
            check,
        }
    }
}
