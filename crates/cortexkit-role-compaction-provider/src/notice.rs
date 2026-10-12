//! Receipts for notices sent in a real provider request. The daemon verifies
//! the runner's identity; that runner, not the compaction provider, observes
//! the successful 2xx response headers and reports the exact request bytes
//! and identities. Transport failures and non-2xx responses are not dispatch.

use std::collections::BTreeMap;

use cortexkit_role_step_transform_provider::claude_code::{ClaudeCodeLane, InvalidParams};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Channel2Directive {
    pub directive_id: String,
    pub text: String,
    pub armed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PublishedView {
    pub view_id: String,
    pub version: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DispatchReceipt {
    pub provider_generation: String,
    pub lineage_id: String,
    pub request_id: String,
    pub dispatch_id: String,
    pub applied_view: PublishedView,
    pub final_wire_sha256: String,
}

pub(crate) fn is_sha256(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Pending notices and accepted receipts for one provider generation and
/// transcript lineage. Save both together in durable storage before replying
/// so restart cannot forget a notice or apply its acknowledgement twice.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct NoticeLedger {
    provider_generation: String,
    lineage_id: String,
    pending: Option<Channel2Directive>,
    receipts: BTreeMap<String, DispatchReceipt>,
}

impl NoticeLedger {
    pub fn new(provider_generation: impl Into<String>, lineage_id: impl Into<String>) -> Self {
        Self {
            provider_generation: provider_generation.into(),
            lineage_id: lineage_id.into(),
            pending: None,
            receipts: BTreeMap::new(),
        }
    }

    fn authorize(
        &self,
        lane: &ClaudeCodeLane,
        principal: &str,
        connection: &str,
    ) -> Result<(), InvalidParams> {
        let bundle = lane
            .selected_for(principal, connection)?
            .ok_or_else(|| InvalidParams::new("channel2_directive"))?;
        if !bundle.notice_dispatch_ack || bundle.provider_generation != self.provider_generation {
            return Err(InvalidParams::new("channel2_directive"));
        }
        Ok(())
    }

    pub fn arm(
        &mut self,
        lane: &ClaudeCodeLane,
        principal: &str,
        connection: &str,
        directive: Channel2Directive,
    ) -> Result<(), InvalidParams> {
        self.authorize(lane, principal, connection)?;
        if directive.directive_id.is_empty()
            || self.receipts.contains_key(&directive.directive_id)
            || self
                .pending
                .as_ref()
                .is_some_and(|pending| pending != &directive)
        {
            return Err(InvalidParams::new("channel2_directive"));
        }
        self.pending = Some(directive);
        Ok(())
    }

    /// No implicit timeout clears a notice. Re-serve these exact bytes until a
    /// matching dispatch acknowledgement is durably recorded.
    pub fn pending(&self) -> Option<&Channel2Directive> {
        self.pending.as_ref()
    }

    pub fn acknowledge(
        &mut self,
        lane: &ClaudeCodeLane,
        principal: &str,
        connection: &str,
        directive_id: &str,
        receipt: DispatchReceipt,
    ) -> Result<DispatchReceipt, InvalidParams> {
        self.authorize(lane, principal, connection)?;
        if receipt.provider_generation != self.provider_generation
            || receipt.lineage_id != self.lineage_id
            || receipt.request_id.is_empty()
            || receipt.dispatch_id.is_empty()
            || receipt.applied_view.view_id.is_empty()
            || !is_sha256(&receipt.final_wire_sha256)
        {
            return Err(InvalidParams::new("channel2_receipt"));
        }
        if let Some(original) = self.receipts.get(directive_id) {
            return if original == &receipt {
                Ok(original.clone())
            } else {
                Err(InvalidParams::new("channel2_receipt"))
            };
        }
        if self
            .pending
            .as_ref()
            .map(|notice| notice.directive_id.as_str())
            != Some(directive_id)
        {
            return Err(InvalidParams::new("channel2_delivered_id"));
        }
        self.receipts
            .insert(directive_id.to_owned(), receipt.clone());
        self.pending = None;
        Ok(receipt)
    }
}
