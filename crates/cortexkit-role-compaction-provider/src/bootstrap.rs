//! Completion evidence for importing original native messages into a fresh
//! provider generation. Before recording completion, the provider must hold all
//! originals and validate that the initial replacement preserves required
//! signed/tool boundaries. Receiving a final page or configuring Setup alone
//! does not establish either property.

use cortexkit_role_step_transform_provider::claude_code::{
    ClaudeCodeCapabilities, ClaudeCodeLane, InvalidParams,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    notice::{is_sha256, PublishedView},
    status::{MessageRef, StepStatus},
};

pub const RECEIPT_FORMAT: &str = "native-bootstrap-receipt/v1";

/// Hash original raw messages in strictly increasing ordinal order. Each
/// message contributes its 8-byte big-endian UTF-8 byte length, then its exact
/// raw bytes. An empty source hashes the empty stream.
pub fn source_sha256(messages: &[crate::status::StatusMessage]) -> Result<String, InvalidParams> {
    let mut hasher = Sha256::new();
    let mut previous = None;
    for message in messages {
        if previous.is_some_and(|ordinal| ordinal >= message.ordinal) {
            return Err(InvalidParams::new("messages"));
        }
        cortexkit_role_step_transform_provider::claude_code::check_raw_message(
            message.raw_message.as_deref(),
        )?;
        let raw = message
            .raw_message
            .as_ref()
            .ok_or_else(|| InvalidParams::new("raw_message"))?
            .as_bytes();
        hasher.update((raw.len() as u64).to_be_bytes());
        hasher.update(raw);
        previous = Some(message.ordinal);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct BootstrapReceipt {
    pub format: String,
    pub resume_id: String,
    pub provider_generation: String,
    pub profile_generation: String,
    pub lineage_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_frontier: Option<MessageRef>,
    pub message_count: u64,
    pub source_sha256: String,
    /// Complete Claude Code capabilities selected at transform.declare,
    /// including profile/generations, epoch/version pins and payload byte caps.
    pub frozen_bundle: ClaudeCodeCapabilities,
    pub initial_view: PublishedView,
}

impl BootstrapReceipt {
    pub fn validate(&self, selected: &ClaudeCodeCapabilities) -> Result<(), InvalidParams> {
        if !selected.bootstrap_complete
            || self.format != RECEIPT_FORMAT
            || self.resume_id.is_empty()
            || self.lineage_id.is_empty()
            || self.provider_generation != selected.provider_generation
            || self.profile_generation != selected.profile_generation
            || &self.frozen_bundle != selected
            || self.initial_view.view_id.is_empty()
            || !is_sha256(&self.source_sha256)
            || (self.message_count == 0) != self.source_frontier.is_none()
            || self
                .source_frontier
                .as_ref()
                .is_some_and(|frontier| frontier.mid.is_empty())
        {
            return Err(InvalidParams::new("bootstrap_complete"));
        }
        Ok(())
    }
}

/// Durable completion state for one provider/profile generation and lineage.
/// Save this ledger with the admitted originals and validated initial view
/// before replying. Restore it only from provider-owned durable records, never
/// request JSON: a caller could otherwise fabricate prior completion and
/// bypass validation of the originals.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct BootstrapLedger {
    provider_generation: String,
    profile_generation: String,
    lineage_id: String,
    receipt: Option<BootstrapReceipt>,
}

impl BootstrapLedger {
    pub fn new(
        provider_generation: impl Into<String>,
        profile_generation: impl Into<String>,
        lineage_id: impl Into<String>,
    ) -> Self {
        Self {
            provider_generation: provider_generation.into(),
            profile_generation: profile_generation.into(),
            lineage_id: lineage_id.into(),
            receipt: None,
        }
    }

    /// Call only after the provider has verified complete, contiguous original
    /// admission, the source digest/count and safe application of the initial
    /// view. This helper cannot recover missing originals or invent that proof.
    pub fn complete(
        &mut self,
        lane: &ClaudeCodeLane,
        principal: &str,
        connection: &str,
        final_status: &StepStatus,
        receipt: BootstrapReceipt,
    ) -> Result<BootstrapReceipt, InvalidParams> {
        let selected = lane
            .selected_for(principal, connection)?
            .ok_or_else(|| InvalidParams::new("bootstrap_complete"))?;
        receipt.validate(selected)?;
        if final_status.more
            || final_status.lineage_id != self.lineage_id
            || receipt.lineage_id != self.lineage_id
            || receipt.provider_generation != self.provider_generation
            || receipt.profile_generation != self.profile_generation
            || final_status.newest.as_ref() != receipt.source_frontier.as_ref()
        {
            return Err(InvalidParams::new("bootstrap_complete"));
        }
        final_status.validate_claude_code(lane, principal, connection)?;
        if let Some(original) = &self.receipt {
            if original != &receipt {
                return Err(InvalidParams::new("bootstrap_complete"));
            }
            return Ok(original.clone());
        }
        self.receipt = Some(receipt.clone());
        Ok(receipt)
    }
}
