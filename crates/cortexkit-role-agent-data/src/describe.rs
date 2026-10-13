//! `agent_data.describe {}`: what this participant holds.
//!
//! The answer names the ref kinds the participant holds, so entorhinal
//! sends each ref only to the participant that declares its kind; the
//! participant's per-agent stores with their sync classes; and whether part
//! of an agent's data is a file another program owns and ships itself. A
//! read.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `describe` request: an empty object. Strict: any member is refused.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DescribeRequest {}

impl DescribeRequest {
    pub fn new() -> Self {
        Self {}
    }
}

/// The `describe` reply: `{ref_kinds, stores, external, detail?}`.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct DescribeReply {
    /// The ref kinds this participant holds.
    pub ref_kinds: Vec<String>,
    /// Its per-agent stores.
    pub stores: Vec<Store>,
    /// True when part of an agent's data is a file another program owns
    /// and ships itself. Today only THALAMUS declares it, for the Claude
    /// Code transcripts behind its harness; a participant whose data door
    /// items can carry (the OpenCode and Pi harnesses among them) declares
    /// `false`.
    pub external: bool,
    /// Optional and opaque: any JSON value is accepted and entorhinal
    /// passes it on without reading it, so a newer participant can add to
    /// it without breaking an older entorhinal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl DescribeReply {
    pub fn new(ref_kinds: Vec<String>, stores: Vec<Store>, external: bool) -> Self {
        Self {
            ref_kinds,
            stores,
            external,
            detail: None,
        }
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Whether this participant lists `ref_kind` in its `ref_kinds`.
    pub fn holds_kind(&self, ref_kind: &str) -> bool {
        self.ref_kinds.iter().any(|kind| kind == ref_kind)
    }
}

/// One per-agent store: `{name, sync_class}`. `sync_class` is the class the
/// CortexKit agent sync design (AGENT_SYNC in CONTRACT.md's Terms) assigns
/// the store. Its values are opaque strings to this crate, which only
/// carries them.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Store {
    pub name: String,
    pub sync_class: String,
}

impl Store {
    pub fn new(name: impl Into<String>, sync_class: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sync_class: sync_class.into(),
        }
    }
}
