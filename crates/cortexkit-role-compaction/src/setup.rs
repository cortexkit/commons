//! `compaction.setup`, served by the provider. The runner calls it when it
//! starts serving a session, after its own restart, before the next send
//! after a Setup that failed with no view applied (W1.3a), and when the
//! provider answers a notice `setup_required` or refuses a ride
//! `unknown_session` (S1). Setup only
//! announces the session; it never carries messages, and the provider reads
//! the transcript afterwards, never inside Setup.
//!
//! The session and its project come from the route, not from the request:
//! the route's bind identity names the session, and the daemon's scope stamp
//! on the route names the session's folder and project.

use serde::{Deserialize, Serialize};

use crate::read::Cursor;
use crate::ride::RideCause;
use crate::rules::ProviderRules;
use crate::view::AppliedView;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SetupRequest {
    /// Authoritative runner-state stamp: replaces the provider's held stamp,
    /// even when lower after a session restore. See W3.5 for restore ordering.
    /// Send only to providers declaring `compaction-statement-seq/v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub statement_seq: Option<u64>,
    /// Runner-authoritative signed reasoning boundary. Send only to providers
    /// declaring `compaction-signed-floor/v1`; see [`crate::SignedFloor`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signed_floor: Option<crate::SignedFloor>,
    /// The session's preset, for example `head`, `worker` or `reader`. The
    /// provider decides what each preset enables.
    pub preset: String,
    /// Harness for child sessions on this runner. Absent means the session's
    /// own harness. Sent only when that host cannot create sessions; never
    /// inferred from runner or host names. A change affects the next child,
    /// not a child already running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_harness: Option<String>,
    pub model: ModelRef,
    /// Whether the session's model reads images. Set only when known; absent
    /// means unknown. A provider inserts images only when this is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts_images: Option<bool>,
    /// Runner opt-in to checksummed `built_through` and `new_signed` view
    /// members. Send true only when verifying both and using `render::plan_view`
    /// (view library 0.1.8 or later). Changes apply to the next view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_anchor: Option<bool>,
    /// Runner-owned read ceilings and window budget (W5.1). The page ceiling
    /// applies to every provider; named-read and window behaviour requires
    /// `compaction-limits/v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<Limits>,
    /// The model's context window in tokens.
    #[serde(deserialize_with = "crate::integer::required")]
    pub context_window: u64,
    /// Tokens the runner keeps free for the reply. The runner never sends a
    /// request larger than `context_window - output_reserve`.
    #[serde(deserialize_with = "crate::integer::required")]
    pub output_reserve: u64,
    pub rules: ProviderRules,
    /// The ride causes the runner can detect. A cause it cannot detect is
    /// unknown to the provider, never "did not happen": a runner that cannot
    /// see an auth plugin's request chains leaves out `chain_reset`.
    pub detects: Vec<RideCause>,
    /// The view the runner applies now, absent when it applies none. After a
    /// runner restart this is the view it reloaded from its own store; for a
    /// session moved in, the first view the move installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_view: Option<AppliedView>,
    /// The newest transcript entry, absent for an empty transcript.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<Cursor>,
    /// The runner's clock, in milliseconds since the Unix epoch.
    #[serde(deserialize_with = "crate::integer::required")]
    pub now: u64,
}

/// The model the session currently uses.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// The provider's service tier (for example OpenAI's `priority`), when it
    /// is not the default. A tier change loses the prompt cache like a model
    /// change does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

/// Runner-owned bounds. An absent member states no bound for that dimension.
/// All integers use the JSON-safe range 0..=9007199254740991; no bound is
/// enabled by default.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Limits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<PageLimits>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub select_max: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowLimits>,
}

/// Read-page ceilings. A page still returns at least one matching entry,
/// including an entry larger than the byte ceiling (W5.4).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PageLimits {
    /// At least 1 when present, so a matching page can always make progress.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::positive")]
    pub max_entries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub max_bytes: Option<u64>,
}

/// Soft budget for the session's window. Exceeding it requests a view, never
/// refuses or ends a step (W3.5).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub entries: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "crate::integer::optional")]
    pub bytes: Option<u64>,
}

impl Limits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_page(mut self, page: PageLimits) -> Self {
        self.page = Some(page);
        self
    }

    pub fn with_select_max(mut self, select_max: u64) -> Self {
        self.select_max = Some(select_max);
        self
    }

    pub fn with_window(mut self, window: WindowLimits) -> Self {
        self.window = Some(window);
        self
    }
}

impl PageLimits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_entries(mut self, max_entries: u64) -> Self {
        self.max_entries = Some(max_entries);
        self
    }

    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = Some(max_bytes);
        self
    }
}

impl WindowLimits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_entries(mut self, entries: u64) -> Self {
        self.entries = Some(entries);
        self
    }

    pub fn with_bytes(mut self, bytes: u64) -> Self {
        self.bytes = Some(bytes);
        self
    }
}

open_tagged! {
/// Setup either answers `ready` or fails with a refusal naming why
/// ([`crate::wire::Refusal`], for example `unknown_preset`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupAnswer, "answer" {
    /// The provider manages the session from now on.
    Ready => "ready",
}
}

impl SetupRequest {
    /// Build the value with its required fields.
    pub fn new(
        preset: impl Into<String>,
        model: ModelRef,
        context_window: u64,
        output_reserve: u64,
        rules: ProviderRules,
        detects: Vec<RideCause>,
        now: u64,
    ) -> Self {
        Self {
            statement_seq: None,
            signed_floor: None,
            preset: preset.into(),
            child_harness: None,
            model,
            accepts_images: None,
            view_anchor: None,
            limits: None,
            context_window,
            output_reserve,
            rules,
            detects,
            now,
            applied_view: None,
            newest: None,
        }
    }

    pub fn with_child_harness(mut self, child_harness: impl Into<String>) -> Self {
        self.child_harness = Some(child_harness.into());
        self
    }

    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Advertise budget detection only to providers declaring `compaction-limits/v1`.
    pub fn with_window_over_budget_detection(mut self, detects: bool, declared: bool) -> Self {
        crate::ride::project_window_over_budget(&mut self.detects, detects, declared);
        self
    }

    /// Project the state stamp only when the provider declares its capability.
    pub fn with_statement_seq(mut self, statement_seq: u64, declared: bool) -> Self {
        self.statement_seq = declared.then_some(statement_seq);
        self
    }

    /// Project the boundary only when the provider declares its capability.
    pub fn with_signed_floor(mut self, floor: crate::SignedFloor, declared: bool) -> Self {
        self.signed_floor = declared.then_some(floor);
        self
    }

    pub fn with_view_anchor(mut self, view_anchor: bool) -> Self {
        self.view_anchor = Some(view_anchor);
        self
    }

    pub fn with_accepts_images(mut self, accepts_images: bool) -> Self {
        self.accepts_images = Some(accepts_images);
        self
    }

    pub fn with_applied_view(mut self, applied_view: AppliedView) -> Self {
        self.applied_view = Some(applied_view);
        self
    }

    pub fn with_newest(mut self, newest: Cursor) -> Self {
        self.newest = Some(newest);
        self
    }
}

impl ModelRef {
    /// Build the value with its required fields.
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variant: None,
            service_tier: None,
        }
    }

    pub fn with_variant(mut self, variant: impl Into<String>) -> Self {
        self.variant = Some(variant.into());
        self
    }

    pub fn with_service_tier(mut self, service_tier: impl Into<String>) -> Self {
        self.service_tier = Some(service_tier.into());
        self
    }
}
