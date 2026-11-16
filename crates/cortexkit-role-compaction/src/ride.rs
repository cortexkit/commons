//! `compaction.ride`, served by the provider. The runner asks for a view now,
//! before a request that rewrites its prompt cache anyway, on an exact count
//! above the usable window, after a length refusal, or when the runner's window
//! budget is crossed. The provider answers
//! within the deadline with a view to apply, or with nothing. A budget ride is
//! soft: it never refuses or ends a step, and does not imply a lost cache.

use serde::{Deserialize, Serialize};

use crate::notice::Conditions;
use crate::view::{AppliedView, View};

/// The runner's own pre-send stop when an exact count still exceeds the usable
/// window after its `over_limit` ride. A runner that cannot count exactly never
/// reports it. A provider's length refusal instead ends the run `error` with
/// `provider_code: context_overflow` and the provider's numbers (CONTRACT.md W3.4).
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
    /// For [`RideCause::OverLimit`]: the model provider's stated count after a
    /// length refusal, or the runner's exact count before sending. Absent when
    /// neither is known; never an estimate.
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
    /// but `over_limit` or `window_over_budget`.
    pub fn cache_lost(&self) -> bool {
        self.causes
            .iter()
            .any(|cause| !matches!(cause, RideCause::OverLimit | RideCause::WindowOverBudget))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum RideCause {
    /// Proven expiry: the runner holds every cache touch since its own start,
    /// warms included, and its clock has passed the newest touch plus that
    /// touch's reported lifetime. Unknown until a touch arrives after start;
    /// never reported for an implicit cache. Stopping warms does not move the
    /// expiry deadline away from the last touch plus its lifetime (RD3).
    /// A serving-model change clears the session's one cache clock; expiry stays
    /// unknown until a touch on the model now serving, even after switching back.
    CacheExpired,
    /// The runner restarts its request chain (OpenAI chained requests).
    ChainReset,
    ModelChange,
    TierChange,
    AccountChange,
    /// The system prompt or the tool definitions changed.
    SurfaceChange,
    /// An exact count exceeds `context_window - output_reserve` before sending,
    /// or the model provider refused the request for length. Never an estimate.
    OverLimit,
    /// The runner's window exceeds its Setup budget. Only for providers
    /// declaring `compaction-limits/v1`; unlike cache changes, this is soft.
    WindowOverBudget,
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
    WindowOverBudget => "window_over_budget",
} }

pub(crate) fn project_window_over_budget(
    causes: &mut Vec<RideCause>,
    enabled: bool,
    declared: bool,
) {
    causes.retain(|cause| *cause != RideCause::WindowOverBudget);
    if enabled && declared {
        causes.push(RideCause::WindowOverBudget);
    }
}

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

    /// Project a budget cause only to declaring providers. The runner controls
    /// first-crossing and quarter-growth scheduling (W3.5); this only sets the
    /// cause. When disabled, another cause must still justify sending the ride.
    pub fn with_window_over_budget(mut self, over_budget: bool, declared: bool) -> Self {
        project_window_over_budget(&mut self.causes, over_budget, declared);
        self
    }
}
