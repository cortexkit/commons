//! `compaction.setup`, served by the provider. The runner calls it when it
//! starts serving a session, after its own restart, and when the provider
//! answers a notice `setup_required` or refuses a ride `unknown_session` (S1). Setup only
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
    /// The session's preset, for example `head`, `worker` or `reader`. The
    /// provider decides what each preset enables.
    pub preset: String,
    pub model: ModelRef,
    /// Whether the session's model reads images. Set only when known; absent
    /// means unknown. A provider inserts images only when this is `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts_images: Option<bool>,
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
            preset: preset.into(),
            model,
            accepts_images: None,
            context_window,
            output_reserve,
            rules,
            detects,
            now,
            applied_view: None,
            newest: None,
        }
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
