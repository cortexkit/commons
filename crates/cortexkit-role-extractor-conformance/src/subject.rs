//! What a module under test supplies: its route, its catalog's served
//! models and other rows, and a view of its stand-in provider.

use async_trait::async_trait;

use cortexkit_role_extractor::describe::ImageLimits;

use crate::route::ExtractorRoute;

/// A model the module serves through the stand-in provider, as the module's
/// catalog and routing state it. The suite compares `role.describe` with
/// this list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServedModel {
    /// The catalog id.
    pub model: String,
    /// The `model` value the module sends the provider for it (Clairic's
    /// is `@cf/cloudflare/clairic`).
    pub provider_model: String,
    /// Whether the catalog prices its input tokens.
    pub priced: bool,
    /// Its image limits; `None` when it takes no images.
    pub images: Option<ImageLimits>,
    /// The provider's per-pass token budget.
    pub pass_tokens: u64,
    /// Its total context; `None` when the provider states none.
    pub context_tokens: Option<u64>,
    /// The provider host its items are sent to.
    pub egress_host: String,
}

/// An extractor module under test.
///
/// Every model it serves must route to a stand-in provider this subject
/// controls. The stand-in answers every call with HTTP 500 and no body: the
/// suite checks the request side, so it never needs an answer. For each call
/// it records the request body it received, exactly, under the `state` the
/// body carries.
#[async_trait]
pub trait ExtractorSubject: Send + Sync {
    type Route: ExtractorRoute;

    /// Open the module's management route.
    async fn route(&self) -> Result<Self::Route, String>;

    /// Every model the module serves, from its catalog.
    fn served_models(&self) -> Vec<ServedModel>;

    /// A catalog id whose row's `kind` is not `extraction`, if the catalog
    /// has one. Without it, the suite cannot ask for that refusal.
    fn non_extraction_model(&self) -> Option<String>;

    /// A catalog id whose row states no `kind`, if the catalog has one.
    fn kindless_model(&self) -> Option<String>;

    /// The request bodies the stand-in received for items whose `state`
    /// was `state`, in the order received, as exact text.
    async fn received(&self, state: &str) -> Vec<String>;
}
