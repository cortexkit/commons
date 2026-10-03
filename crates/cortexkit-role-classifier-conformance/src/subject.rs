//! What a module under test supplies: its route, its catalog's served
//! models, and control of its stand-in provider.

use async_trait::async_trait;
use serde_json::Value;

use crate::route::ClassifierRoute;

/// How many times the suite pauses while it waits for something (a held
/// call to arrive, or a call it expects to be refused at once) before it
/// gives up.
pub const MAX_POLLS: usize = 400;

/// What the stand-in provider answers for an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scripted {
    /// HTTP 200 with this exact body: a provider response
    /// `{model, answers, usage?}` as JSON text. The stand-in sends these
    /// bytes unchanged, so the suite can check that the module passes the
    /// provider's number spellings through.
    Answer(String),
    /// This HTTP status with no body (neither provider documents an error
    /// body), and a `Retry-After` of `retry_after_ms` when present.
    Status {
        status: u16,
        retry_after_ms: Option<u64>,
    },
}

impl Scripted {
    pub fn status(status: u16) -> Self {
        Self::Status {
            status,
            retry_after_ms: None,
        }
    }
}

/// A model the module serves through the stand-in provider, as the module's
/// catalog states it. The suite compares `role.describe` with this list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServedModel {
    /// The catalog id.
    pub model: String,
    /// Whether the catalog prices it.
    pub priced: bool,
    /// The catalog's image limit per item; `None` when it takes no images.
    pub images_max: Option<u64>,
    /// The catalog's context, in tokens.
    pub context_tokens: u64,
    /// The host the module sends item text to for this model.
    pub egress_host: String,
}

/// The key the stand-in counts and scripts an item by: the RFC 8785 (JCS)
/// canonical JSON of the `state` it received. The suite gives every item a
/// unique state, so the key names one item, and canonicalizing makes an
/// object state's key order irrelevant.
pub fn state_key(state: &Value) -> String {
    String::from_utf8(serde_jcs::to_vec(state).expect("a JSON value canonicalizes"))
        .expect("JCS output is UTF-8")
}

/// A classifier module under test.
///
/// Every model it serves must route to a stand-in provider this subject
/// controls. For each provider call, the stand-in computes
/// [`state_key`] of the request's `state`, counts the call under it, and
/// answers with what the suite last scripted for that key (an unscripted
/// key answers HTTP 500). When the suite asked it to hold the next call for
/// a key, that one call waits until the suite releases it; later calls for
/// the key answer at once.
#[async_trait]
pub trait ClassifierSubject: Send + Sync {
    type Route: ClassifierRoute;

    /// Open the module's management route.
    async fn route(&self) -> Result<Self::Route, String>;

    /// Every model the module serves, from its catalog.
    fn served_models(&self) -> Vec<ServedModel>;

    /// A catalog id whose row's `kind` is not `classifier`, if the catalog
    /// has one. Without it, the suite cannot ask for `model_not_classifier`.
    fn non_classifier_model(&self) -> Option<String>;

    /// Script what the stand-in answers for `key` from now on.
    async fn script(&self, key: &str, scripted: Scripted);

    /// How many calls the stand-in has received for `key`.
    async fn calls(&self, key: &str) -> usize;

    /// Whether the stand-in can hold a call until released. Without it, the
    /// concurrent-call check is not applicable.
    fn can_hold_calls(&self) -> bool;

    /// Hold the next call for `key` until [`ClassifierSubject::release`].
    async fn hold_next(&self, key: &str);

    /// Whether a call for `key` is being held now.
    async fn is_held(&self, key: &str) -> bool;

    /// Let a held call for `key` answer.
    async fn release(&self, key: &str);

    /// Wait a short while. The suite calls this between polls.
    async fn pause(&self);
}
