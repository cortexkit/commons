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
    /// The catalog's context, in tokens; `None` when the catalog states no
    /// exact figure, in which case `role.describe` must leave it absent.
    pub context_tokens: Option<u64>,
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

/// A model and account returned by the routing stand-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoutedDecision {
    pub model: String,
    pub account: String,
    pub decision_id: String,
}

/// One answer, refusal, or transport failure from the routing stand-in.
#[derive(Clone, Debug)]
pub enum RoutingOutcome {
    Selected(RoutedDecision),
    Refused { code: String, message: String },
    Unavailable,
}

/// An answered routing refusal, including detail such as a retry timestamp.
/// Use the constructor and setters so future fixture members stay additive.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct RoutingRefusalFixture {
    pub code: String,
    pub message: String,
    pub detail: Value,
}

impl RoutingRefusalFixture {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: serde_json::json!({}),
        }
    }

    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = detail;
        self
    }
}

/// Observations made by stand-ins, not inferred from the classifier reply.
#[derive(Clone, Debug, Default)]
pub struct RoutingObservations {
    pub selections: usize,
    /// The frozen decision actually used for each provider request, in order.
    pub provider_requests: Vec<RoutedDecision>,
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

    /// A catalog id for a row that states no `kind`, if the catalog contains
    /// such a row. Without it, the suite cannot check the kindless-row refusal.
    fn kindless_model(&self) -> Option<String> {
        None
    }

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

    /// The image-reference hook: hand `bytes`, of `content_type`, to the
    /// image source the module resolves references through (the stand-in
    /// for the module that issues handles), and return the handle the
    /// module will resolve to them. `None`, the default, when the subject
    /// has no such hook: the checks that need a resolvable handle are then
    /// not applicable. A subject that returns a handle must also implement
    /// [`ClassifierSubject::set_image_source_down`].
    async fn issue_image_reference(&self, content_type: &str, bytes: Vec<u8>) -> Option<String> {
        let _ = (content_type, bytes);
        None
    }

    /// Make the image source unreachable (`true`), so the module's
    /// resolution of any handle fails as if the issuing module were down or
    /// timed out, or reachable again (`false`).
    async fn set_image_source_down(&self, down: bool) {
        let _ = down;
    }

    /// Script routing in order and reset its observations. After the last
    /// outcome, repeat that outcome. Return false when no routing hook is
    /// available; the routing checks then report not applicable.
    async fn script_routing(&self, outcomes: Vec<RoutingOutcome>) -> bool {
        let _ = outcomes;
        false
    }

    /// Refuse admission with this answered routing refusal. Reset routing
    /// observations and repeat the refusal until `finish_routing`. Return
    /// false when unsupported, making the case not applicable.
    async fn script_routing_admission_refusal(&self, refusal: RoutingRefusalFixture) -> bool {
        let _ = refusal;
        false
    }

    /// With a selection script active, force lease renewal after the first
    /// answered item and before any later dispatch; answer renewal with this
    /// refusal. Do not select a new decision. Return false when unsupported.
    async fn script_routing_renewal(&self, refusal: RoutingRefusalFixture) -> bool {
        let _ = refusal;
        false
    }

    /// Count actual routing selections and record model/account/decision
    /// used by actual provider requests while a routing script is active.
    async fn routing_observations(&self) -> Option<RoutingObservations> {
        None
    }

    /// Restore normal routing after a scripted case, including failed cases.
    async fn finish_routing(&self) {}
}
