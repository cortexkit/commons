//! `compaction.ride`, served by the provider. The runner asks for a view now,
//! just before a request that rewrites its prompt cache anyway, or that would
//! not fit. The provider answers within the deadline with a view to apply, or
//! with nothing. The wait only ever lands on a request that is already paying
//! a full cache write, or that would otherwise be refused.

use serde::{Deserialize, Serialize};

use crate::notice::Conditions;
use crate::view::{AppliedView, View};

/// The refusal code a runner gives a run whose next request still does not
/// fit after an `over_limit` ride. It never sends a request over its limit.
pub const CONTEXT_CEILING: &str = "context_ceiling";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RideRequest {
    /// Every cause that applies, at least one, none twice. An expired cache
    /// and an over-limit request together tell the provider both that the
    /// rewrite is paid for and that the request must shrink.
    pub causes: Vec<RideCause>,
    /// How long the runner waits for the answer, in milliseconds. A cold
    /// start gets a longer deadline, since the provider may first have to
    /// read the transcript.
    #[serde(deserialize_with = "crate::integer::required")]
    pub deadline_ms: u64,
    pub conditions: Conditions,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_view: Option<AppliedView>,
    /// For [`RideCause::OverLimit`]: the runner's estimate, in tokens, of the
    /// request it is about to send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub request_tokens: Option<u64>,
}

impl RideRequest {
    /// Whether `cause` is among the ride's causes.
    pub fn has(&self, cause: RideCause) -> bool {
        self.causes.contains(&cause)
    }

    /// Whether the model provider's cache is lost for this request: any cause
    /// but `over_limit`.
    pub fn cache_lost(&self) -> bool {
        self.causes
            .iter()
            .any(|cause| *cause != RideCause::OverLimit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum RideCause {
    /// The prompt cache is proven expired: by the runner's own clock, its last
    /// send (keep-warm requests included) is older than the cache lifetime it
    /// requested. A runner that cannot tell, for example after its own restart
    /// without a durable record of that send, sends no `cache_expired` (RD3).
    CacheExpired,
    /// The runner restarts its request chain (OpenAI chained requests).
    ChainReset,
    ModelChange,
    TierChange,
    AccountChange,
    /// The system prompt or the tool definitions changed.
    SurfaceChange,
    /// The next request would exceed `context_window - output_reserve`.
    OverLimit,
    /// An unrecognised wire value, kept verbatim; handled as CONTRACT.md
    /// W1.5 says.
    Unknown(String),
}

open_string! { RideCause {
    CacheExpired => "cache_expired",
    ChainReset => "chain_reset",
    ModelChange => "model_change",
    TierChange => "tier_change",
    AccountChange => "account_change",
    SurfaceChange => "surface_change",
    OverLimit => "over_limit",
} }

open_tagged! {
#[derive(Clone, Debug, PartialEq)]
pub enum RideAnswer, "answer" {
    /// Apply this view to the coming request.
    View => "view" { view: View },
    /// Keep the current view.
    Nothing => "nothing",
}
}

impl RideRequest {
    /// Build the value with its required fields.
    pub fn new(causes: Vec<RideCause>, deadline_ms: u64, conditions: Conditions) -> Self {
        Self {
            causes,
            deadline_ms,
            conditions,
            applied_view: None,
            request_tokens: None,
        }
    }

    pub fn with_applied_view(mut self, applied_view: AppliedView) -> Self {
        self.applied_view = Some(applied_view);
        self
    }

    pub fn with_request_tokens(mut self, request_tokens: u64) -> Self {
        self.request_tokens = Some(request_tokens);
        self
    }
}
