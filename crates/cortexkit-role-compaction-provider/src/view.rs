//! Immutable raw replacements and optional, byte-cursor view retrieval.

use cortexkit_role_step_transform_provider::claude_code::{ClaudeCodeLane, MAX_ENVELOPE_BYTES};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// An inclusive original native top-level content part. Scalar content has
/// exactly part zero; nested tool-result children are not coordinates here.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NativeEndpoint {
    pub message_id: String,
    pub part_index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ViewRef {
    pub view_id: String,
    pub version: u64,
    pub byte_length: u64,
    pub sha256: String,
}

/// Raw UTF-8 JSON bytes, preserved without parse/re-encode, or an immutable
/// locator served by `compaction.view.read`. Bytes are a JSON string on wire.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RawPayload {
    Inline { bytes: String },
    Reference { view: ViewRef },
}

/// A retained native part's immutable engine identity after a hook cut.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RetainedPart {
    pub endpoint: NativeEndpoint,
    pub engine_handle: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct FrozenView {
    #[serde(flatten)]
    pub view: ViewRef,
    pub profile: String,
    pub profile_generation: String,
    pub provider_generation: String,
    pub lineage_id: String,
    pub endpoint: NativeEndpoint,
    pub payload: RawPayload,
    /// Exact head for a native hook cut, not the whole role replacement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cut_replacement_ref: Option<ViewRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retained_tail: Option<Vec<RetainedPart>>,
}

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

impl ViewRef {
    pub fn check_bytes(&self, bytes: &[u8]) -> Result<(), ViewReadRefusal> {
        if self.byte_length != bytes.len() as u64 || self.sha256 != sha256(bytes) {
            return Err(ViewReadRefusal::DigestMismatch);
        }
        Ok(())
    }
}

impl FrozenView {
    pub fn validate(&self) -> Result<(), ViewReadRefusal> {
        if self.view.view_id.is_empty()
            || self.endpoint.message_id.is_empty()
            || !crate::notice::is_sha256(&self.view.sha256)
            || self.profile.is_empty()
            || self.profile_generation.is_empty()
            || self.provider_generation.is_empty()
            || self.lineage_id.is_empty()
            || self.cut_replacement_ref.as_ref().is_some_and(|head| {
                head.view_id.is_empty() || !crate::notice::is_sha256(&head.sha256)
            })
            || self.retained_tail.as_ref().is_some_and(|tail| {
                tail.iter().any(|part| {
                    part.endpoint.message_id.is_empty() || part.engine_handle.is_empty()
                })
            })
            || self.cut_replacement_ref.is_some() != self.retained_tail.is_some()
        {
            return Err(ViewReadRefusal::InvalidView);
        }
        match &self.payload {
            RawPayload::Inline { bytes } => self.view.check_bytes(bytes.as_bytes()),
            RawPayload::Reference { view } if view == &self.view => Ok(()),
            RawPayload::Reference { .. } => Err(ViewReadRefusal::InvalidView),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ViewReadRequest {
    pub request_id: String,
    pub view_id: String,
    pub version: u64,
    pub sha256: String,
    pub byte_cursor: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ViewReadRefusal {
    UnknownVersion,
    DigestMismatch,
    InvalidCursor,
    InvalidView,
    UnavailableLane,
    EnvelopeTooLarge,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum ViewReadAnswer {
    Page {
        request_id: String,
        #[serde(flatten)]
        view: ViewRef,
        byte_cursor: u64,
        bytes: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        next_byte_cursor: Option<u64>,
    },
    Refuse {
        request_id: String,
        code: ViewReadRefusal,
    },
}

/// Read only the supplied immutable object. Before calling this helper, look
/// it up within the authenticated connection's frozen provider generation and
/// transcript lineage (CONTRACT.md §19.3); never use a global latest-view lookup.
/// The runner may apply the inline replacement without ever calling this op.
pub fn read_frozen_view(
    lane: &ClaudeCodeLane,
    principal: &str,
    connection: &str,
    view: &FrozenView,
    raw_bytes: &str,
    request: &ViewReadRequest,
) -> Result<ViewReadAnswer, ViewReadRefusal> {
    let bundle = lane
        .selected_for(principal, connection)
        .map_err(|_| ViewReadRefusal::UnavailableLane)?
        .filter(|bundle| bundle.raw_view_read)
        .ok_or(ViewReadRefusal::UnavailableLane)?;
    if view.provider_generation != bundle.provider_generation
        || view.profile_generation != bundle.profile_generation
        || view.profile != bundle.profile
    {
        return Err(ViewReadRefusal::InvalidView);
    }
    if request.view_id != view.view.view_id || request.version != view.view.version {
        return Err(ViewReadRefusal::UnknownVersion);
    }
    view.validate()?;
    view.view.check_bytes(raw_bytes.as_bytes())?;
    if request.sha256 != view.view.sha256 {
        return Err(ViewReadRefusal::DigestMismatch);
    }
    let cursor =
        usize::try_from(request.byte_cursor).map_err(|_| ViewReadRefusal::InvalidCursor)?;
    if cursor > raw_bytes.len() || !raw_bytes.is_char_boundary(cursor) {
        return Err(ViewReadRefusal::InvalidCursor);
    }
    let mut end = cursor
        .saturating_add(bundle.caps.view_page_bytes)
        .min(raw_bytes.len());
    while !raw_bytes.is_char_boundary(end) {
        end -= 1;
    }
    loop {
        let answer = ViewReadAnswer::Page {
            request_id: request.request_id.clone(),
            view: view.view.clone(),
            byte_cursor: request.byte_cursor,
            bytes: raw_bytes[cursor..end].to_owned(),
            next_byte_cursor: (end < raw_bytes.len()).then_some(end as u64),
        };
        if serde_json::to_vec(&answer)
            .map_err(|_| ViewReadRefusal::EnvelopeTooLarge)?
            .len()
            < MAX_ENVELOPE_BYTES
        {
            if end == cursor && cursor != raw_bytes.len() {
                return Err(ViewReadRefusal::EnvelopeTooLarge);
            }
            return Ok(answer);
        }
        if end == cursor {
            return Err(ViewReadRefusal::EnvelopeTooLarge);
        }
        end = cursor + (end - cursor) / 2;
        while !raw_bytes.is_char_boundary(end) {
            end -= 1;
        }
    }
}
