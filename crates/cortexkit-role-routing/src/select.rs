//! `route.select`: the request, its validator, and the answer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};

use crate::{
    capabilities::SELECT_HOLD_CONTROL,
    errors::{PinUnavailableReason, Refusal},
    members::Members,
};
/// Every requirement member this version defines, in contract order.
/// Providers list the members they read in `requirement_fields`.
pub const REQUIREMENT_FIELDS: &[&str] = &[
    "iq",
    "eq",
    "context_window",
    "max_output_tokens",
    "tools",
    "tool_capabilities",
    "modalities",
    "structured_output",
    "questions",
    "input_tokens",
    "max_cost_usd",
];

/// Requirement members callers send only when the router lists them.
pub const OPTIONAL_REQUIREMENT_FIELDS: &[&str] = &["questions", "input_tokens", "max_cost_usd"];

/// The highest `iq` or `eq` a request may state.
pub const DEMAND_MAX: u8 = 100;
/// The longest `affinity` key, in characters.
pub const AFFINITY_MAX_CHARS: usize = 128;
/// The longest `correlation_id`, in characters.
pub const CORRELATION_ID_MAX_CHARS: usize = 128;
/// The longest `explanation`, in characters.
pub const EXPLANATION_MAX_CHARS: usize = 200;
/// The most entries a `prefer` list may carry.
pub const PREFER_MAX_ENTRIES: usize = 16;

open_set! {
    /// The `tools` requirement.
    pub enum ToolsRequirement {
        /// Only runner and model pairs that run with no tools offered.
        None => "none",
        /// Only routes that accept tool calls.
        Required => "required",
    }
}

open_set! {
    /// An input modality the work sends.
    pub enum Modality {
        Image => "image",
        Pdf => "pdf",
        Audio => "audio",
    }
}

open_set! {
    /// What to do when the request's pin cannot be served. The only value
    /// this version reads is `"route"`: route normally instead of refusing
    /// `pin_unavailable`. Any other value is `invalid_request` naming
    /// `pin_fallback`.
    pub enum PinFallback {
        /// Route normally when the pin cannot be served.
        Route => "route",
    }
}

open_set! {
    /// How a pinned select was answered. Carried whenever the request pinned:
    /// `"pinned"` when the pin was served, `"fell_back"` when
    /// `pin_fallback: "route"` routed instead. On `fell_back` the answer's
    /// `detail` carries `pin_reason` naming the pin refusal's reason.
    pub enum PinOutcome {
        /// The pin was served.
        Pinned => "pinned",
        /// The pin could not be served and the provider routed instead.
        FellBack => "fell_back",
    }
}

open_set! {
    /// How the router resolved the request's `settings.project_id`.
    pub enum ProjectSettings {
        /// The project's settings were applied.
        Applied => "applied",
        /// The project has no settings.
        None => "none",
        /// The project could not be found; selection proceeds without its settings.
        Unresolved => "unresolved",
    }
}

/// Settings lookup overrides, sent only with the `select_settings` group.
/// These are never scope: pins, cost attribution and consumer ownership still
/// follow the route's stamp. No member names a folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectSettings {
    /// Identity-service project id. A router must refuse a value different
    /// from a project stamp on the route as `settings.project_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Consumer whose service-tier settings apply, 1 to 128 bytes. Changes
    /// only that lookup, not the request's consumer or ownership.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_consumer: Option<String>,
}

impl SelectSettings {
    /// Use the route's ordinary settings lookups.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_project_id(mut self, project_id: impl Into<String>) -> Self {
        self.project_id = Some(project_id.into());
        self
    }

    pub fn with_tier_consumer(mut self, tier_consumer: impl Into<String>) -> Self {
        self.tier_consumer = Some(tier_consumer.into());
        self
    }

    /// Check the project id against the route's trusted daemon scope stamp.
    /// The router must call this on every scoped route, passing the project id
    /// from its bind's stamp, never from the request. It is not called on
    /// unscoped routes. Project ids are compared as exact strings, not normalized.
    pub fn check_project_stamp(&self, stamp_project_id: Option<&str>) -> Result<(), Refusal> {
        if let (Some(project_id), Some(stamp_project_id)) = (&self.project_id, stamp_project_id) {
            if project_id != stamp_project_id {
                return Err(Refusal::invalid_request(
                    "settings.project_id",
                    "settings.project_id differs from the route's project stamp",
                ));
            }
        }
        Ok(())
    }
}

/// What the work demands of a route.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Requirements {
    /// Task demand, 0 to 100, mapped by the provider onto its own model
    /// facts.
    pub iq: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eq: Option<u8>,
    /// Floor, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u64>,
    /// Floor, in tokens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Absent means no constraint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsRequirement>,
    /// Tool capability tags (such as `code.read/v1`) the work's plan lists.
    /// Only runners that can reach a tool provider serving every one of
    /// these tags for the scoped project are answered. Tags, never module
    /// names: any provider serving the same tags can stand in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_capabilities: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modalities: Option<Vec<Modality>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<bool>,
    /// The most questions in any single provider request; at least one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub questions: Option<u64>,
    /// Estimated total input tokens for the decision. Zero means no estimate
    /// and is allowed only without a cost cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    /// Positive input-spend cap; requires a positive `input_tokens` estimate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<Number>,
    /// Members this version does not define, kept so a provider can refuse
    /// them by name instead of losing them in a decode.
    #[serde(flatten, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Value>,
}

impl Requirements {
    /// Requirements demanding only `iq`.
    pub fn iq(iq: u8) -> Self {
        Self {
            iq,
            ..Self::default()
        }
    }
}

/// A one-slot concurrency hint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcurrencySlot {
    pub key: String,
    pub running: u64,
    pub limit: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<u64>,
}

/// The `route.select` request.
/// Build with [`Self::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectRequest {
    /// Send only to a router declaring `service_tier`. Absent means standard.
    /// Only accounts whose plans include the chosen tier are eligible.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::service_tier::deserialize_request"
    )]
    pub service_tier: Option<crate::service_tier::ServiceTier>,
    /// A consumer id the provider knows from configuration or declarations.
    pub consumer: String,
    pub requirements: Requirements,
    /// `provider/model[:variant]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    /// Route keys (`provider/model`) the caller will not take; each
    /// excludes every variant of that key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,
    /// Provider ids (the part of a route key before the `/`) the caller will
    /// not take: no route from a listed provider is answered, whatever its
    /// model or variant. Sent only to a provider declaring
    /// [`crate::capabilities::SELECT_EXCLUDE_PROVIDERS`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_providers: Option<Vec<String>>,
    /// Runner module ids the caller can send to. Absent: any runner the
    /// provider knows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runners: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affinity: Option<String>,
    /// Echoed in logs only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Opaque task reference for decision attribution and per-task cost profiles,
    /// never selection. 1–128 bytes, each printable ASCII `0x21..=0x7e`.
    /// Sent only to a provider declaring [`crate::capabilities::SELECT_TASK_REF`].
    /// Stored with the decision, never returned in an answer or to another caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub concurrency_slots: Option<Vec<ConcurrencySlot>>,
    /// Asks for the provider's explanation object.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<bool>,
    /// `false`: do not answer a blocked entry while another entry is usable
    /// now; when none is, the fallback may still hold. Absent means `true`.
    /// Sent only to a provider declaring
    /// [`crate::capabilities::SELECT_HOLD_CONTROL`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_ok: Option<bool>,
    /// The caller's own cap on a hold, in seconds; `0` means no hold. Sent
    /// only to a provider declaring
    /// [`crate::capabilities::SELECT_HOLD_CONTROL`]. Valid with
    /// `hold_ok: false` too: the fallback that answer can take may hold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold_wait_max_s: Option<u64>,
    /// Exact session program; sent only to a provider declaring
    /// [`crate::capabilities::SELECT_HARNESS`]. Never selects a wire/auth pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// Ordered route preference: at most [`PREFER_MAX_ENTRIES`] entries, no
    /// duplicates, each `provider/model` without a variant. Among the routes
    /// the provider would already answer for the request it answers a listed
    /// one when one is adequate, in list order. It never admits a route the
    /// request would not otherwise get. Sent only to a provider declaring
    /// [`crate::capabilities::SELECT_PREFER`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer: Option<Vec<String>>,
    /// `pin_fallback: "route"` beside `pin` or `pin_slot`: when the pin cannot be served,
    /// route normally instead of refusing `pin_unavailable`. Without it the
    /// pin still refuses and a second select for the same work stays
    /// forbidden. Any other value is `invalid_request` naming the field, and
    /// a `pin_fallback` without either pin member is too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_fallback: Option<PinFallback>,
    /// Operator-policy slot, a token sent only with `select_pin_slot`.
    /// Mutually exclusive with `pin`. An unknown slot is `pin_unavailable`
    /// naming the slot with reason `unknown_slot`; a known slot's unavailable
    /// route has the same reason as a `pin` of that route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_slot: Option<String>,
    /// Non-empty route-key list (`provider/model`, no variants) the caller's
    /// runners can reach. Narrows independently of `runners` and `exclude`;
    /// repeated entries are allowed. Send only with `select_available_routes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_routes: Option<Vec<String>>,
    /// Prompt-cache preference, sent only with `select_prefer_prompt_cache`.
    /// Breaks ties among adequate routes, never admits a route.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefer_prompt_cache: Option<bool>,
    /// Settings lookups, sent only with `select_settings`; never scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SelectSettings>,
}

impl SelectRequest {
    /// A request for `consumer` with `requirements` and nothing else.
    pub fn new(consumer: impl Into<String>, requirements: Requirements) -> Self {
        Self {
            consumer: consumer.into(),
            service_tier: None,
            requirements,
            pin: None,
            exclude: None,
            exclude_providers: None,
            runners: None,
            affinity: None,
            correlation_id: None,
            task_ref: None,
            concurrency_slots: None,
            detail: None,
            hold_ok: None,
            hold_wait_max_s: None,
            harness: None,
            prefer: None,
            pin_fallback: None,
            pin_slot: None,
            available_routes: None,
            prefer_prompt_cache: None,
            settings: None,
        }
    }

    pub fn with_harness(mut self, harness: impl Into<String>) -> Self {
        self.harness = Some(harness.into());
        self
    }

    pub fn with_service_tier(mut self, tier: crate::service_tier::ServiceTier) -> Self {
        self.service_tier = Some(tier);
        self
    }

    /// Set the pin, `provider/model[:variant]`.
    pub fn with_pin(mut self, pin: impl Into<String>) -> Self {
        self.pin = Some(pin.into());
        self
    }

    pub fn with_pin_slot(mut self, pin_slot: impl Into<String>) -> Self {
        self.pin_slot = Some(pin_slot.into());
        self
    }

    pub fn with_available_routes(mut self, available_routes: Vec<String>) -> Self {
        self.available_routes = Some(available_routes);
        self
    }

    pub fn with_prefer_prompt_cache(mut self, prefer_prompt_cache: bool) -> Self {
        self.prefer_prompt_cache = Some(prefer_prompt_cache);
        self
    }

    pub fn with_settings(mut self, settings: SelectSettings) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Set what to do when the pin cannot be served.
    pub fn with_pin_fallback(mut self, pin_fallback: PinFallback) -> Self {
        self.pin_fallback = Some(pin_fallback);
        self
    }

    /// Set the ordered route preference, each `provider/model`.
    pub fn with_prefer(mut self, prefer: Vec<String>) -> Self {
        self.prefer = Some(prefer);
        self
    }

    /// Set the route keys the caller will not take.
    pub fn with_exclude(mut self, exclude: Vec<String>) -> Self {
        self.exclude = Some(exclude);
        self
    }

    /// Set the provider ids none of whose routes the caller will take.
    pub fn with_exclude_providers(mut self, exclude_providers: Vec<String>) -> Self {
        self.exclude_providers = Some(exclude_providers);
        self
    }

    /// Whether the request's `exclude_providers` lists `provider`.
    pub fn excludes_provider(&self, provider: &str) -> bool {
        self.exclude_providers
            .as_ref()
            .is_some_and(|excluded| excluded.iter().any(|listed| listed == provider))
    }

    /// Set the runner module ids the caller can send to.
    pub fn with_runners(mut self, runners: Vec<String>) -> Self {
        self.runners = Some(runners);
        self
    }

    /// Set the affinity key.
    pub fn with_affinity(mut self, affinity: impl Into<String>) -> Self {
        self.affinity = Some(affinity.into());
        self
    }

    /// Set the correlation id the provider echoes in its logs.
    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }

    pub fn with_task_ref(mut self, task_ref: impl Into<String>) -> Self {
        self.task_ref = Some(task_ref.into());
        self
    }

    /// Set the concurrency hint.
    pub fn with_concurrency_slots(mut self, concurrency_slots: Vec<ConcurrencySlot>) -> Self {
        self.concurrency_slots = Some(concurrency_slots);
        self
    }

    /// Ask, or decline to ask, for the provider's explanation object.
    pub fn with_detail(mut self, detail: bool) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Set `hold_ok`: whether a blocked entry may be answered while another
    /// entry is usable now.
    pub fn with_hold_ok(mut self, hold_ok: bool) -> Self {
        self.hold_ok = Some(hold_ok);
        self
    }

    /// Set the caller's own cap on a hold, in seconds.
    pub fn with_hold_wait_max_s(mut self, hold_wait_max_s: u64) -> Self {
        self.hold_wait_max_s = Some(hold_wait_max_s);
        self
    }

    /// `hold_ok` with its default (`true`) applied.
    pub fn hold_ok(&self) -> bool {
        self.hold_ok.unwrap_or(true)
    }
}

/// A model on a provider, with an optional variant.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RouteRef {
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

impl RouteRef {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            variant: None,
        }
    }

    /// The route key: `provider/model`, without the variant.
    pub fn route_key(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }

    /// The pin text: `provider/model[:variant]`.
    pub fn pin(&self) -> String {
        match &self.variant {
            Some(variant) => format!("{}/{}:{variant}", self.provider, self.model),
            None => self.route_key(),
        }
    }

    /// Parse a pin, `provider/model[:variant]`. The provider is everything
    /// before the first `/`; a variant is what follows the last `:` after
    /// it. `None` when any part is empty.
    pub fn parse_pin(pin: &str) -> Option<Self> {
        let (provider, rest) = pin.split_once('/')?;
        let (model, variant) = match rest.rsplit_once(':') {
            Some((model, variant)) => (model, Some(variant)),
            None => (rest, None),
        };
        if provider.is_empty() || model.is_empty() || variant.is_some_and(str::is_empty) {
            return None;
        }
        Some(Self {
            provider: provider.to_owned(),
            model: model.to_owned(),
            variant: variant.map(str::to_owned),
        })
    }
}

/// Whether `key` has the shape of a route key: `provider/model`, both
/// non-empty.
pub fn is_route_key(key: &str) -> bool {
    matches!(key.split_once('/'), Some((provider, model)) if is_provider_id(provider) && !model.is_empty())
}

/// Whether `key` is a route key without a variant: [`is_route_key`], with no
/// `:` in its model part. A pin reads a variant after the last `:` past the
/// first `/` ([`RouteRef::parse_pin`]), so only the model part is checked; a
/// provider id may contain `:`.
pub fn is_plain_route_key(key: &str) -> bool {
    is_route_key(key)
        && key
            .split_once('/')
            .is_some_and(|(_, model)| !model.contains(':'))
}

/// Whether `id` has the shape of a route key's provider part: non-empty and
/// without a `/`, since a route key's provider is everything before its
/// first `/`.
pub fn is_provider_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/')
}

/// The key a runner sends with, named by public id only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerAuth {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
}

/// Where the work runs.
/// Build with [`Self::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RunnerRef {
    /// The runner module id; one of the request's `runners` when it sent
    /// them.
    pub module: String,
    /// The model as the runner names it.
    pub model: RouteRef,
    /// Present only for providers whose policy names a key pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<RunnerAuth>,
    /// The session program: a requested program the runner declares, otherwise
    /// its declared default or sole program. Never inferred from a route/pair.
    /// A caller does not bind a runner session on an answer without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
}

impl RunnerRef {
    /// `module` running `model`, with no key pool and no harness.
    pub fn new(module: impl Into<String>, model: RouteRef) -> Self {
        Self {
            module: module.into(),
            model,
            auth: None,
            harness: None,
        }
    }

    /// Set the key the runner sends with.
    pub fn with_auth(mut self, auth: RunnerAuth) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Set the harness the runner declared for this route.
    pub fn with_harness(mut self, harness: impl Into<String>) -> Self {
        self.harness = Some(harness.into());
        self
    }
}

/// The `route.select` answer.
/// Build with [`Self::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SelectAnswer {
    /// The level selected after ultrafast -> fast -> standard fallback. Present
    /// for an explicit tier request, including a fallback to standard. Producers
    /// use ServiceTier::as_str; readers retain unfamiliar record levels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier_chosen: Option<String>,
    pub decision_id: String,
    pub route: RouteRef,
    pub runner: RunnerRef,
    /// From the provider's own catalog read, in tokens.
    pub context_window: u64,
    /// From the provider's own catalog read, in tokens.
    pub max_output_tokens: u64,
    pub expires_at_ms: u64,
    /// One sentence, at most [`EXPLANATION_MAX_CHARS`] characters.
    pub explanation: String,
    /// The provider's non-normative explanation object, when asked for.
    /// On `pin_outcome: "fell_back"` it carries `pin_reason` naming the
    /// pin refusal's reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
    /// How a pinned select was answered. Present exactly when the request
    /// pinned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_outcome: Option<PinOutcome>,
    /// Present exactly when the request sent `settings.project_id`. An
    /// unresolved project never refuses selection; it has no project settings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_settings: Option<ProjectSettings>,
}

impl SelectAnswer {
    /// An answer naming `route` on `runner` with the provider's limits.
    pub fn new(
        decision_id: impl Into<String>,
        route: RouteRef,
        runner: RunnerRef,
        context_window: u64,
        max_output_tokens: u64,
        expires_at_ms: u64,
        explanation: impl Into<String>,
    ) -> Self {
        Self {
            decision_id: decision_id.into(),
            service_tier_chosen: None,
            route,
            runner,
            context_window,
            max_output_tokens,
            expires_at_ms,
            explanation: explanation.into(),
            detail: None,
            pin_outcome: None,
            project_settings: None,
        }
    }

    /// Set the provider's explanation object.
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn with_service_tier_chosen(mut self, tier: crate::service_tier::ServiceTier) -> Self {
        self.service_tier_chosen = Some(tier.as_str().into());
        self
    }

    /// Set how a pinned select was answered.
    pub fn with_pin_outcome(mut self, pin_outcome: PinOutcome) -> Self {
        self.pin_outcome = Some(pin_outcome);
        self
    }

    pub fn with_project_settings(mut self, project_settings: ProjectSettings) -> Self {
        self.project_settings = Some(project_settings);
        self
    }
}

/// Validate and decode `route.select` params for a provider that declares
/// no optional group: [`parse_select_request_serving`] with none.
pub fn parse_select_request(params: &Value) -> Result<SelectRequest, Refusal> {
    parse_select_request_serving::<&str>(params, &[])
}

/// Validate and decode `route.select` params for a provider that declares
/// `capabilities` in `role.describe`.
///
/// A malformed requirement is refused `invalid_requirements` naming it, and
/// so is a requirement this version does not define: it is never ignored.
/// A malformed pin is `pin_unavailable` with reason `invalid`. Any other
/// bad member is `invalid_request` naming it. `hold_ok` and
/// `hold_wait_max_s` are refused by name unless `capabilities` lists
/// [`SELECT_HOLD_CONTROL`]. `harness` is refused by name unless the provider
/// declares [`crate::capabilities::SELECT_HARNESS`]. `prefer` is refused by
/// name unless the provider declares [`crate::capabilities::SELECT_PREFER`],
/// and `exclude_providers` unless it declares
/// [`crate::capabilities::SELECT_EXCLUDE_PROVIDERS`]. `task_ref` is refused
/// by name unless the provider declares [`crate::capabilities::SELECT_TASK_REF`].
/// `pin_slot`, `available_routes`, `prefer_prompt_cache` and `settings` require
/// `select_pin_slot`, `select_available_routes`, `select_prefer_prompt_cache`
/// and `select_settings`, respectively.
/// Whether the consumer is known and owned, and
/// whether anything can serve, is the provider's to answer after this.
pub fn parse_select_request_serving<C: AsRef<str>>(
    params: &Value,
    capabilities: &[C],
) -> Result<SelectRequest, Refusal> {
    let members = Members::of(params)?;
    for (field, group) in [
        ("pin_slot", "select_pin_slot"),
        ("available_routes", "select_available_routes"),
        ("prefer_prompt_cache", "select_prefer_prompt_cache"),
        ("settings", "select_settings"),
    ] {
        if members
            .object
            .get(field)
            .is_some_and(|value| !value.is_null())
            && !capabilities
                .iter()
                .any(|capability| capability.as_ref() == group)
        {
            return Err(members.refuse(field, format!("{field} requires the {group} group")));
        }
    }
    let consumer = members.text("consumer", None)?;
    let service_tier = crate::service_tier::parse_member(members.object.get("service_tier"))?;
    if service_tier.is_some()
        && !capabilities
            .iter()
            .any(|capability| capability.as_ref() == crate::capabilities::SERVICE_TIER)
    {
        return Err(members.refuse(
            "service_tier",
            "service_tier requires the service_tier group",
        ));
    }
    let requirements = parse_requirements(members.object.get("requirements"))?;
    let pin = members.optional_text("pin", None)?;
    let pin_slot = members.optional_text("pin_slot", None)?;
    if let Some(slot) = &pin_slot {
        if pin.is_some() {
            return Err(members.refuse("pin_slot", "pin_slot and pin are mutually exclusive"));
        }
        if !crate::declarations::is_runner_token(slot) {
            return Err(members.refuse("pin_slot", "pin_slot must be a token"));
        }
    }
    if let Some(pin) = &pin {
        if RouteRef::parse_pin(pin).is_none() {
            return Err(Refusal::pin_unavailable(
                pin.clone(),
                PinUnavailableReason::Invalid,
                None,
            ));
        }
    }
    let available_routes: Option<Vec<String>> = members.optional("available_routes")?;
    if let Some(routes) = &available_routes {
        if routes.is_empty() || routes.iter().any(|key| !is_plain_route_key(key)) {
            return Err(members.refuse(
                "available_routes",
                "available_routes must be a non-empty list of route keys provider/model",
            ));
        }
    }
    let prefer_prompt_cache = members.optional("prefer_prompt_cache")?;
    let settings = parse_settings(members.object.get("settings"))?;
    let exclude: Option<Vec<String>> = members.optional("exclude")?;
    if let Some(exclude) = &exclude {
        if let Some(bad) = exclude.iter().find(|key| !is_plain_route_key(key)) {
            return Err(members.refuse(
                "exclude",
                format!("{bad:?} is not a route key provider/model"),
            ));
        }
    }
    let exclude_providers: Option<Vec<String>> = members.optional("exclude_providers")?;
    if let Some(exclude_providers) = &exclude_providers {
        // Without the group the member is refused, never ignored: a caller
        // that sent it must not be answered a route it meant to rule out.
        if !capabilities
            .iter()
            .any(|capability| capability.as_ref() == crate::capabilities::SELECT_EXCLUDE_PROVIDERS)
        {
            return Err(members.refuse(
                "exclude_providers",
                format!(
                    "exclude_providers is read only by a provider declaring {}",
                    crate::capabilities::SELECT_EXCLUDE_PROVIDERS
                ),
            ));
        }
        if let Some(bad) = exclude_providers.iter().find(|id| !is_provider_id(id)) {
            return Err(members.refuse(
                "exclude_providers",
                format!("{bad:?} is not a provider id, the provider part of a route key"),
            ));
        }
    }
    let runners: Option<Vec<String>> = members.optional("runners")?;
    if runners
        .as_ref()
        .is_some_and(|runners| runners.iter().any(String::is_empty))
    {
        return Err(members.refuse("runners", "a runner id must not be empty"));
    }
    let hold_ok: Option<bool> = members.optional("hold_ok")?;
    let harness = members.optional_text("harness", None)?;
    if let Some(harness) = &harness {
        if !crate::declarations::is_runner_token(harness) {
            return Err(members.refuse("harness", "harness must be a runner token"));
        }
        if !capabilities
            .iter()
            .any(|capability| capability.as_ref() == crate::capabilities::SELECT_HARNESS)
        {
            return Err(members.refuse(
                "harness",
                format!(
                    "harness is read only by a provider declaring {}",
                    crate::capabilities::SELECT_HARNESS
                ),
            ));
        }
    }
    let task_ref = members.optional_text("task_ref", None)?;
    if let Some(task_ref) = &task_ref {
        if task_ref.len() > 128 || !task_ref.bytes().all(|byte| (0x21..=0x7e).contains(&byte)) {
            return Err(members.refuse(
                "task_ref",
                "task_ref must be 1 to 128 bytes, each printable ASCII 0x21-0x7e",
            ));
        }
        if !capabilities
            .iter()
            .any(|capability| capability.as_ref() == crate::capabilities::SELECT_TASK_REF)
        {
            return Err(members.refuse(
                "task_ref",
                format!(
                    "task_ref is read only by a provider declaring {}",
                    crate::capabilities::SELECT_TASK_REF
                ),
            ));
        }
    }
    let hold_wait_max_s: Option<u64> = members.optional("hold_wait_max_s")?;
    let hold_control = capabilities
        .iter()
        .any(|capability| capability.as_ref() == SELECT_HOLD_CONTROL);
    if !hold_control {
        // A provider that does not declare the group refuses its members
        // exactly as it refuses any member it does not read, so a caller
        // never mistakes an ignored cap for an honoured one.
        for (field, present) in [
            ("hold_ok", hold_ok.is_some()),
            ("hold_wait_max_s", hold_wait_max_s.is_some()),
        ] {
            if present {
                return Err(members.refuse(
                    field,
                    format!("{field} is read only by a provider declaring {SELECT_HOLD_CONTROL}"),
                ));
            }
        }
    }
    let pin_fallback: Option<PinFallback> = members.optional("pin_fallback")?;
    if let Some(fallback) = &pin_fallback {
        if !fallback.is_known() {
            return Err(members.refuse(
                "pin_fallback",
                format!("{fallback} is not a pin fallback this provider reads"),
            ));
        }
        if pin.is_none() && pin_slot.is_none() {
            return Err(members.refuse("pin_fallback", "pin_fallback needs pin or pin_slot"));
        }
    }
    let prefer: Option<Vec<String>> = members.optional("prefer")?;
    if let Some(prefer) = &prefer {
        if !capabilities
            .iter()
            .any(|capability| capability.as_ref() == crate::capabilities::SELECT_PREFER)
        {
            return Err(members.refuse(
                "prefer",
                format!(
                    "prefer is read only by a provider declaring {}",
                    crate::capabilities::SELECT_PREFER
                ),
            ));
        }
        if prefer.len() > PREFER_MAX_ENTRIES {
            return Err(members.refuse(
                "prefer",
                format!("prefer carries more than {PREFER_MAX_ENTRIES} entries"),
            ));
        }
        let mut seen = std::collections::BTreeSet::new();
        for entry in prefer {
            if !seen.insert(entry) {
                return Err(
                    members.refuse("prefer", format!("{entry:?} is listed twice in prefer"))
                );
            }
            if !is_plain_route_key(entry) {
                return Err(members.refuse(
                    "prefer",
                    format!("{entry:?} is not a route key provider/model"),
                ));
            }
        }
    }
    Ok(SelectRequest {
        consumer,
        service_tier,
        requirements,
        pin: pin.clone(),
        exclude,
        exclude_providers,
        runners,
        affinity: members.optional_text("affinity", Some(AFFINITY_MAX_CHARS))?,
        correlation_id: members.optional_text("correlation_id", Some(CORRELATION_ID_MAX_CHARS))?,
        task_ref,
        concurrency_slots: members.optional("concurrency_slots")?,
        detail: members.optional("detail")?,
        hold_ok,
        hold_wait_max_s,
        harness,
        prefer,
        pin_fallback,
        pin_slot,
        available_routes,
        prefer_prompt_cache,
        settings,
    })
}

pub(crate) fn parse_settings(value: Option<&Value>) -> Result<Option<SelectSettings>, Refusal> {
    let object = match value {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Object(object)) => object,
        _ => {
            return Err(Refusal::invalid_request(
                "settings",
                "settings must be an object",
            ))
        }
    };
    let members = Members {
        object,
        refuse: |field, message| Refusal::invalid_request(format!("settings.{field}"), message),
    };
    for field in object.keys() {
        if !matches!(field.as_str(), "project_id" | "tier_consumer") {
            return Err(members.refuse(field, "unknown settings member"));
        }
    }
    let project_id = members.optional_text("project_id", None)?;
    let tier_consumer = members.optional_text("tier_consumer", None)?;
    if tier_consumer
        .as_ref()
        .is_some_and(|consumer| consumer.len() > 128)
    {
        return Err(members.refuse("tier_consumer", "tier_consumer must be 1 to 128 bytes"));
    }
    Ok(Some(SelectSettings {
        project_id,
        tier_consumer,
    }))
}

pub(crate) fn parse_requirements(value: Option<&Value>) -> Result<Requirements, Refusal> {
    let Some(Value::Object(object)) = value else {
        return Err(Refusal::invalid_requirements(
            "requirements",
            "requirements must be an object",
        ));
    };
    if let Some(unknown) = object
        .keys()
        .find(|key| !REQUIREMENT_FIELDS.contains(&key.as_str()))
    {
        return Err(Refusal::invalid_requirements(
            unknown.clone(),
            format!("{unknown} is not a requirement this provider reads"),
        ));
    }
    let members = Members {
        object,
        refuse: |field, message| Refusal::invalid_requirements(field, message),
    };
    let demand = |field: &str, value: Option<u8>| match value {
        Some(value) if value > DEMAND_MAX => {
            Err(members.refuse(field, format!("{field} is above {DEMAND_MAX}")))
        }
        _ => Ok(value),
    };
    let iq = demand("iq", Some(members.required("iq")?))?.unwrap_or_default();
    let eq = demand("eq", members.optional("eq")?)?;
    let tools: Option<ToolsRequirement> = members.optional("tools")?;
    if tools.as_ref().is_some_and(|tools| !tools.is_known()) {
        return Err(members.refuse("tools", "tools must be \"none\" or \"required\""));
    }
    let tool_capabilities: Option<Vec<String>> = members.optional("tool_capabilities")?;
    if tool_capabilities
        .as_ref()
        .is_some_and(|tags| tags.iter().any(String::is_empty))
    {
        return Err(members.refuse("tool_capabilities", "a tag must not be empty"));
    }
    let modalities: Option<Vec<Modality>> = members.optional("modalities")?;
    if let Some(unknown) = modalities
        .iter()
        .flatten()
        .find(|modality| !modality.is_known())
    {
        return Err(members.refuse(
            "modalities",
            format!("{unknown} is not a modality this provider reads"),
        ));
    }
    let questions: Option<u64> = members.optional("questions")?;
    if questions == Some(0) {
        return Err(members.refuse("questions", "questions must be at least 1"));
    }
    let input_tokens: Option<u64> = members.optional("input_tokens")?;
    let max_cost_usd: Option<Number> = members.optional("max_cost_usd")?;
    if let Some(cap) = &max_cost_usd {
        if !cap.as_f64().is_some_and(|cap| cap > 0.0) {
            return Err(members.refuse("max_cost_usd", "max_cost_usd must be positive"));
        }
        if input_tokens.unwrap_or_default() == 0 {
            return Err(members.refuse(
                "input_tokens",
                "a cost cap requires input_tokens at least 1",
            ));
        }
    }
    Ok(Requirements {
        iq,
        eq,
        context_window: members.optional("context_window")?,
        max_output_tokens: members.optional("max_output_tokens")?,
        tools,
        tool_capabilities,
        modalities,
        structured_output: members.optional("structured_output")?,
        questions,
        input_tokens,
        max_cost_usd,
        unknown: BTreeMap::new(),
    })
}

/// Why a `route.select` answer breaks a rule a consumer can check alone.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AnswerProblem {
    /// An explicit tier request lacks its chosen level, the chosen level is
    /// unrecognised, or the answer upgrades the request. This check is stricter
    /// than record decoding, which retains unfamiliar strings.
    ServiceTier,
    /// The answer's runner is not one of the request's `runners`.
    RunnerNotListed(String),
    /// The answer's route key is not in the request's `available_routes`.
    RouteNotAvailable(String),
    /// The answer's route key is in the request's `exclude`.
    Excluded(String),
    /// The answer's route key (held here) is from a provider the request's
    /// `exclude_providers` lists.
    ExcludedProvider(String),
    /// The explanation is longer than [`EXPLANATION_MAX_CHARS`].
    ExplanationTooLong(usize),
    /// The request pinned but the answer carries no `pin_outcome`.
    MissingPinOutcome,
    /// The request did not pin but the answer carries a `pin_outcome`.
    UnexpectedPinOutcome,
    /// The request pinned and fell back but the answer's `detail` carries no
    /// `pin_reason`.
    MissingPinReason,
    /// The request sent `settings.project_id` but the answer carries no result.
    MissingProjectSettings,
    /// No `settings.project_id` was sent but the answer carries a result.
    UnexpectedProjectSettings,
}

impl std::fmt::Display for AnswerProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ServiceTier => write!(
                f,
                "service_tier_chosen is missing, unrecognised or exceeds the requested tier"
            ),
            Self::RunnerNotListed(runner) => {
                write!(f, "runner {runner:?} is not one of the request's runners")
            }
            Self::RouteNotAvailable(key) => {
                write!(f, "route {key:?} is not in the request's available_routes")
            }
            Self::Excluded(key) => write!(f, "route {key:?} is in the request's exclude"),
            Self::ExcludedProvider(key) => write!(
                f,
                "route {key:?} is from a provider in the request's exclude_providers"
            ),
            Self::ExplanationTooLong(chars) => write!(
                f,
                "the explanation is {chars} characters, over {EXPLANATION_MAX_CHARS}"
            ),
            Self::MissingPinOutcome => {
                write!(
                    f,
                    "the request pinned but the answer carries no pin_outcome"
                )
            }
            Self::UnexpectedPinOutcome => {
                write!(
                    f,
                    "the request did not pin but the answer carries a pin_outcome"
                )
            }
            Self::MissingPinReason => {
                write!(
                    f,
                    "the answer fell back but its detail carries no pin_reason"
                )
            }
            Self::MissingProjectSettings => {
                write!(
                    f,
                    "settings.project_id was sent but project_settings is missing"
                )
            }
            Self::UnexpectedProjectSettings => {
                write!(
                    f,
                    "project_settings was answered without settings.project_id"
                )
            }
        }
    }
}

/// Check an answer against the request it answers: the runner is one the
/// caller listed, the route is not one it excluded nor from a provider it
/// excluded, the explanation is within its bound, and a pinned request is
/// answered with a `pin_outcome` (with `pin_reason` in `detail` on
/// `fell_back`). A chosen tier must be recognised and may not exceed a
/// requested tier; without one, the router's tier settings may choose any
/// level. This is stricter than record decoding, which retains unknown tiers.
/// The route must also be in `available_routes` when sent, and
/// `project_settings` must be present exactly when `settings.project_id` was sent.
/// Pin rules apply to `pin_slot` just as to `pin`.
pub fn check_answer(request: &SelectRequest, answer: &SelectAnswer) -> Result<(), AnswerProblem> {
    if !crate::service_tier::check_chosen(
        request.service_tier,
        answer.service_tier_chosen.as_deref(),
    ) {
        return Err(AnswerProblem::ServiceTier);
    }
    if let Some(runners) = &request.runners {
        if !runners.contains(&answer.runner.module) {
            return Err(AnswerProblem::RunnerNotListed(answer.runner.module.clone()));
        }
    }
    let key = answer.route.route_key();
    if request
        .available_routes
        .as_ref()
        .is_some_and(|routes| !routes.contains(&key))
    {
        return Err(AnswerProblem::RouteNotAvailable(key));
    }
    if request
        .exclude
        .as_ref()
        .is_some_and(|exclude| exclude.contains(&key))
    {
        return Err(AnswerProblem::Excluded(key));
    }
    if request.excludes_provider(&answer.route.provider) {
        return Err(AnswerProblem::ExcludedProvider(key));
    }
    let chars = answer.explanation.chars().count();
    if chars > EXPLANATION_MAX_CHARS {
        return Err(AnswerProblem::ExplanationTooLong(chars));
    }
    match (
        request.pin.is_some() || request.pin_slot.is_some(),
        &answer.pin_outcome,
    ) {
        (true, None) => return Err(AnswerProblem::MissingPinOutcome),
        (false, Some(_)) => return Err(AnswerProblem::UnexpectedPinOutcome),
        (true, Some(outcome)) if *outcome == PinOutcome::FellBack => {
            let reason = answer
                .detail
                .as_ref()
                .and_then(Value::as_object)
                .and_then(|detail| detail.get("pin_reason"))
                .and_then(Value::as_str);
            if reason.is_none_or(str::is_empty) {
                return Err(AnswerProblem::MissingPinReason);
            }
        }
        _ => {}
    }
    let project_sent = request
        .settings
        .as_ref()
        .is_some_and(|settings| settings.project_id.is_some());
    match (project_sent, answer.project_settings.is_some()) {
        (true, false) => return Err(AnswerProblem::MissingProjectSettings),
        (false, true) => return Err(AnswerProblem::UnexpectedProjectSettings),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errors::RefusalCode;
    use serde_json::json;

    fn field(refusal: &Refusal) -> &str {
        refusal.detail["field"].as_str().unwrap()
    }

    #[test]
    fn pins_parse_with_and_without_a_variant() {
        let pin = RouteRef::parse_pin("example-provider/model-a:fast").unwrap();
        assert_eq!(pin.route_key(), "example-provider/model-a");
        assert_eq!(pin.variant.as_deref(), Some("fast"));
        assert_eq!(pin.pin(), "example-provider/model-a:fast");
        assert!(RouteRef::parse_pin("example-provider/model-a")
            .unwrap()
            .variant
            .is_none());
        for bad in ["model-a", "/m", "p/", "p/m:", ""] {
            assert!(RouteRef::parse_pin(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn an_unknown_requirement_is_refused_by_name() {
        let refusal = parse_select_request(
            &json!({"consumer": "flow", "requirements": {"iq": 50, "gpu_count": 2}}),
        )
        .unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequirements);
        assert_eq!(field(&refusal), "gpu_count");
    }

    #[test]
    fn decision_input_requirements_validate_and_keep_unknowns() {
        let valid = serde_json::json!({"iq": 0, "questions": 3,
            "input_tokens": 1000, "max_cost_usd": 0.01});
        let parsed = parse_requirements(Some(&valid)).unwrap();
        assert_eq!(parsed.questions, Some(3));
        assert_eq!(parsed.input_tokens, Some(1000));
        assert_eq!(
            parsed.max_cost_usd,
            Some(serde_json::Number::from_f64(0.01).unwrap())
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), valid);
        for (addition, expected) in [
            (serde_json::json!({"questions": 0}), "questions"),
            (serde_json::json!({"questions": 1.5}), "questions"),
            (serde_json::json!({"input_tokens": -1}), "input_tokens"),
            (serde_json::json!({"input_tokens": 1.5}), "input_tokens"),
            (serde_json::json!({"max_cost_usd": 0}), "max_cost_usd"),
            (serde_json::json!({"max_cost_usd": -1}), "max_cost_usd"),
            (serde_json::json!({"max_cost_usd": "1"}), "max_cost_usd"),
            (serde_json::json!({"max_cost_usd": 1}), "input_tokens"),
            (
                serde_json::json!({"max_cost_usd": 1, "input_tokens": 0}),
                "input_tokens",
            ),
        ] {
            let mut requirements = serde_json::json!({"iq": 0});
            requirements
                .as_object_mut()
                .unwrap()
                .extend(addition.as_object().unwrap().clone());
            assert_eq!(
                field(&parse_requirements(Some(&requirements)).unwrap_err()),
                expected
            );
        }
        for value in [
            serde_json::json!({"iq":0}),
            serde_json::json!({"iq":0,"input_tokens":0}),
        ] {
            assert!(parse_requirements(Some(&value)).is_ok());
        }
        let unknown = serde_json::json!({"iq":0,"future_requirement":1});
        let decoded: Requirements = serde_json::from_value(unknown.clone()).unwrap();
        assert_eq!(decoded.unknown["future_requirement"], 1);
        assert_eq!(
            field(&parse_requirements(Some(&unknown)).unwrap_err()),
            "future_requirement"
        );
    }

    #[test]
    fn a_decoded_request_keeps_unknown_requirements() {
        let request: SelectRequest = serde_json::from_value(
            json!({"consumer": "flow", "requirements": {"iq": 50, "gpu_count": 2}}),
        )
        .unwrap();
        assert_eq!(request.requirements.unknown["gpu_count"], json!(2));
    }

    #[test]
    fn bad_members_are_refused_by_name() {
        let cases = [
            (
                json!({"requirements": {"iq": 1}}),
                "invalid_request",
                "consumer",
            ),
            (
                json!({"consumer": "flow"}),
                "invalid_requirements",
                "requirements",
            ),
            (
                json!({"consumer": "flow", "requirements": {}}),
                "invalid_requirements",
                "iq",
            ),
            (
                json!({"consumer": "flow", "requirements": {"iq": 101}}),
                "invalid_requirements",
                "iq",
            ),
            (
                json!({"consumer": "flow", "requirements": {"iq": 1, "tools": "some"}}),
                "invalid_requirements",
                "tools",
            ),
            (
                json!({"consumer": "flow", "requirements": {"iq": 1, "modalities": ["smell"]}}),
                "invalid_requirements",
                "modalities",
            ),
            (
                json!({"consumer": "flow", "requirements": {"iq": 1}, "exclude": ["model-a"]}),
                "invalid_request",
                "exclude",
            ),
            (
                json!({"consumer": "flow", "requirements": {"iq": 1}, "affinity": "x".repeat(129)}),
                "invalid_request",
                "affinity",
            ),
        ];
        for (params, code, expected) in cases {
            let refusal = parse_select_request(&params).unwrap_err();
            assert_eq!(refusal.code.as_str(), code, "{params}");
            assert_eq!(field(&refusal), expected, "{params}");
        }
    }

    #[test]
    fn a_malformed_pin_is_pin_unavailable_invalid() {
        let refusal = parse_select_request(
            &json!({"consumer": "flow", "requirements": {"iq": 1}, "pin": "model-a"}),
        )
        .unwrap_err();
        assert_eq!(refusal.code, RefusalCode::PinUnavailable);
        assert_eq!(refusal.detail["reason"], "invalid");
        assert_eq!(refusal.wait_until_ms(), None);
    }

    #[test]
    fn null_members_read_as_absent() {
        let with_nulls = parse_select_request(&json!({
            "consumer": "flow",
            "requirements": {"iq": 1, "eq": null},
            "pin": null,
            "exclude": null,
            "exclude_providers": null,
            "detail": null,
            "pin_fallback": null,
            "prefer": null,
            "task_ref": null,
        }))
        .unwrap();
        let without =
            parse_select_request(&json!({"consumer": "flow", "requirements": {"iq": 1}})).unwrap();
        assert_eq!(with_nulls, without);
        assert_eq!(
            serde_json::to_value(&without).unwrap(),
            json!({"consumer": "flow", "requirements": {"iq": 1}})
        );
    }

    #[test]
    fn task_ref_accepts_printable_ascii_within_byte_limits() {
        let printable: String = (0x21..=0x7e).map(char::from).collect();
        for task_ref in [
            "!".into(),
            "~".into(),
            "x".repeat(128),
            "wi:ct_1:0/s17-2.#".into(),
            printable,
        ] {
            let params =
                json!({"consumer": "flow", "requirements": {"iq": 1}, "task_ref": task_ref});
            let request =
                parse_select_request_serving(&params, &[crate::capabilities::SELECT_TASK_REF])
                    .unwrap();
            assert_eq!(request.task_ref.as_deref(), Some(task_ref.as_str()));
            assert_eq!(serde_json::to_value(&request).unwrap(), params);
        }
    }

    #[test]
    fn task_ref_invalid_values_are_refused_by_name() {
        for task_ref in [
            json!(""),
            json!("x".repeat(129)),
            json!("has space"),
            json!("wi\u{1f}1"),
            json!("wi\u{7f}1"),
            json!("wi_é"),
            json!(12),
        ] {
            let params =
                json!({"consumer": "flow", "requirements": {"iq": 1}, "task_ref": task_ref});
            let refusal =
                parse_select_request_serving(&params, &[crate::capabilities::SELECT_TASK_REF])
                    .unwrap_err();
            assert_eq!(refusal.code, RefusalCode::InvalidRequest, "{params}");
            assert_eq!(field(&refusal), "task_ref", "{params}");
        }
    }

    #[test]
    fn task_ref_requires_the_declared_group() {
        let refusal = parse_select_request(
            &json!({"consumer": "flow", "requirements": {"iq": 1}, "task_ref": "wi_1"}),
        )
        .unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(field(&refusal), "task_ref");
    }

    fn select_with(additions: Value) -> Value {
        let mut params = json!({"consumer": "flow", "requirements": {"iq": 50}});
        params
            .as_object_mut()
            .unwrap()
            .extend(additions.as_object().unwrap().clone());
        params
    }

    fn answer() -> SelectAnswer {
        let route = RouteRef::new("example-provider", "model-a");
        SelectAnswer::new(
            "decision-1",
            route.clone(),
            RunnerRef::new("runner-one", route),
            128_000,
            8_000,
            1000,
            "Selected an adequate route.",
        )
    }

    fn invalid_addition(additions: Value, groups: &[&str], expected: &str) {
        let refusal =
            parse_select_request_serving(&select_with(additions.clone()), groups).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequest, "{additions}");
        assert_eq!(field(&refusal), expected, "{additions}");
    }

    #[test]
    fn select_additions_require_their_own_groups_and_null_means_absent() {
        for (name, value, group) in [
            ("pin_slot", json!("primary"), "select_pin_slot"),
            (
                "available_routes",
                json!(["example-provider/model-a"]),
                "select_available_routes",
            ),
            (
                "prefer_prompt_cache",
                json!(false),
                "select_prefer_prompt_cache",
            ),
            ("settings", json!({}), "select_settings"),
        ] {
            let additions = json!({name: value});
            invalid_addition(additions.clone(), &[], name);
            invalid_addition(additions.clone(), &["another_group"], name);
            let params = select_with(additions);
            let legacy = parse_select_request(&params).unwrap_err();
            assert_eq!(
                legacy,
                parse_select_request_serving::<&str>(&params, &[]).unwrap_err()
            );
            let request = parse_select_request_serving(&params, &[group]).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(), params);
            assert_eq!(
                parse_select_request(&select_with(json!({name: null}))).unwrap(),
                SelectRequest::new("flow", Requirements::iq(50))
            );
        }
    }

    #[test]
    fn pin_slot_uses_the_token_grammar_and_cannot_accompany_pin() {
        for slot in [
            "a".to_owned(),
            "primary.model-1_slot".to_owned(),
            "a".repeat(64),
        ] {
            let request = parse_select_request_serving(
                &select_with(json!({"pin_slot": slot})),
                &["select_pin_slot"],
            )
            .unwrap();
            assert_eq!(request.pin_slot.as_deref(), Some(slot.as_str()));
        }
        for slot in [
            json!(""),
            json!("a".repeat(65)),
            json!("Primary"),
            json!("1slot"),
            json!("with space"),
            json!("p/m"),
            json!("p:m"),
            json!("é"),
            json!(1),
            json!([]),
        ] {
            invalid_addition(json!({"pin_slot": slot}), &["select_pin_slot"], "pin_slot");
        }
        invalid_addition(
            json!({"pin": "example-provider/model-a", "pin_slot": "primary"}),
            &["select_pin_slot"],
            "pin_slot",
        );
        assert!(parse_select_request_serving(
            &select_with(json!({"pin": null, "pin_slot": "primary"})),
            &["select_pin_slot"]
        )
        .is_ok());
    }

    #[test]
    fn pin_fallback_needs_either_pin_member_and_only_accepts_route() {
        for absent in [json!({}), json!({"pin": null, "pin_slot": null})] {
            let mut additions = absent;
            additions["pin_fallback"] = json!("route");
            invalid_addition(additions, &["select_pin_slot"], "pin_fallback");
        }
        for (name, pin) in [("pin", "example-provider/model-a"), ("pin_slot", "primary")] {
            let request = parse_select_request_serving(
                &select_with(json!({name: pin, "pin_fallback": "route"})),
                &["select_pin_slot"],
            )
            .unwrap();
            assert_eq!(request.pin_fallback, Some(PinFallback::Route));
            for invalid in [json!("other"), json!(false)] {
                invalid_addition(
                    json!({name: pin, "pin_fallback": invalid}),
                    &["select_pin_slot"],
                    "pin_fallback",
                );
            }
        }
    }

    #[test]
    fn pin_slot_refusal_names_the_slot_and_preserves_route_reasons() {
        for reason in [
            PinUnavailableReason::UnknownSlot,
            PinUnavailableReason::UnknownModel,
            PinUnavailableReason::Excluded,
            PinUnavailableReason::Backoff,
        ] {
            let refusal = Refusal::pin_unavailable("primary", reason.clone(), None);
            assert_eq!(refusal.code, RefusalCode::PinUnavailable);
            assert_eq!(refusal.detail["pin"], "primary");
            assert_eq!(refusal.detail["reason"], reason.as_str());
        }
        assert_eq!(
            PinUnavailableReason::parse("unknown_slot"),
            PinUnavailableReason::UnknownSlot
        );
    }

    #[test]
    fn answer_pin_rules_apply_equally_to_pins_and_slots() {
        let plain = SelectRequest::new("flow", Requirements::iq(50));
        assert_eq!(check_answer(&plain, &answer()), Ok(()));
        for request in [
            plain.clone().with_pin("example-provider/model-a"),
            plain.clone().with_pin_slot("primary"),
        ] {
            assert_eq!(
                check_answer(&request, &answer()),
                Err(AnswerProblem::MissingPinOutcome)
            );
            for outcome in [PinOutcome::Pinned, PinOutcome::Other("future".into())] {
                assert_eq!(
                    check_answer(&request, &answer().with_pin_outcome(outcome)),
                    Ok(())
                );
            }
            let fallback = answer().with_pin_outcome(PinOutcome::FellBack);
            assert_eq!(
                check_answer(&request, &fallback),
                Err(AnswerProblem::MissingPinReason)
            );
            for detail in [
                json!(null),
                json!({}),
                json!({"pin_reason": ""}),
                json!({"pin_reason": 1}),
            ] {
                assert_eq!(
                    check_answer(&request, &fallback.clone().with_detail(detail)),
                    Err(AnswerProblem::MissingPinReason)
                );
            }
            assert_eq!(
                check_answer(
                    &request,
                    &fallback.with_detail(json!({"pin_reason": "unknown_slot"}))
                ),
                Ok(())
            );
        }
        for outcome in [
            PinOutcome::Pinned,
            PinOutcome::FellBack,
            PinOutcome::Other("future".into()),
        ] {
            assert_eq!(
                check_answer(&plain, &answer().with_pin_outcome(outcome)),
                Err(AnswerProblem::UnexpectedPinOutcome)
            );
        }
    }

    #[test]
    fn available_routes_is_nonempty_route_keys_with_duplicates_allowed() {
        let group = ["select_available_routes"];
        for routes in [
            json!([]),
            json!([""]),
            json!(["model-a"]),
            json!(["/model-a"]),
            json!(["p/"]),
            json!(["p/m:variant"]),
            json!(["p/m", 7]),
            json!("p/m"),
        ] {
            invalid_addition(
                json!({"available_routes": routes}),
                &group,
                "available_routes",
            );
        }
        for routes in [
            vec!["p/m".to_owned(), "p/m".to_owned()],
            (0..40).map(|n| format!("p/m{n}")).collect(),
        ] {
            let request = parse_select_request_serving(
                &select_with(json!({"available_routes": routes})),
                &group,
            )
            .unwrap();
            assert_eq!(request.available_routes, Some(routes));
        }
    }

    #[test]
    fn answer_available_routes_is_independent_of_runner_and_exclude() {
        let request = SelectRequest::new("flow", Requirements::iq(50))
            .with_available_routes(vec!["example-provider/model-a".into()])
            .with_runners(vec!["runner-one".into()]);
        let mut selected = answer();
        selected.route.variant = Some("fast".into());
        assert_eq!(check_answer(&request, &selected), Ok(()));
        selected.route.model = "model-b".into();
        assert_eq!(
            check_answer(&request, &selected),
            Err(AnswerProblem::RouteNotAvailable(
                "example-provider/model-b".into()
            ))
        );
        let mut selected = answer();
        selected.runner.module = "runner-two".into();
        assert_eq!(
            check_answer(&request, &selected),
            Err(AnswerProblem::RunnerNotListed("runner-two".into()))
        );
        assert_eq!(
            check_answer(
                &request
                    .clone()
                    .with_exclude(vec!["example-provider/model-a".into()]),
                &answer()
            ),
            Err(AnswerProblem::Excluded("example-provider/model-a".into()))
        );
        assert_eq!(
            check_answer(
                &request.with_exclude_providers(vec!["example-provider".into()]),
                &answer()
            ),
            Err(AnswerProblem::ExcludedProvider(
                "example-provider/model-a".into()
            ))
        );
    }

    #[test]
    fn prefer_prompt_cache_is_a_bool_and_does_not_change_answer_checks() {
        for value in [true, false] {
            let request = parse_select_request_serving(
                &select_with(json!({"prefer_prompt_cache": value})),
                &["select_prefer_prompt_cache"],
            )
            .unwrap();
            assert_eq!(request.prefer_prompt_cache, Some(value));
            assert_eq!(check_answer(&request, &answer()), Ok(()));
        }
        for value in [json!(1), json!("true"), json!({})] {
            invalid_addition(
                json!({"prefer_prompt_cache": value}),
                &["select_prefer_prompt_cache"],
                "prefer_prompt_cache",
            );
        }
    }

    #[test]
    fn settings_reject_unknown_members_and_name_invalid_nested_fields() {
        for (settings, expected) in [
            (json!(1), "settings"),
            (json!([]), "settings"),
            (json!({"folder": "/project"}), "settings.folder"),
            (json!({"future": null}), "settings.future"),
            (json!({"project_id": 3}), "settings.project_id"),
            (json!({"project_id": ""}), "settings.project_id"),
            (json!({"tier_consumer": false}), "settings.tier_consumer"),
            (json!({"tier_consumer": ""}), "settings.tier_consumer"),
            (
                json!({"tier_consumer": "a".repeat(129)}),
                "settings.tier_consumer",
            ),
            (
                json!({"tier_consumer": "é".repeat(65)}),
                "settings.tier_consumer",
            ),
        ] {
            invalid_addition(
                json!({"settings": settings}),
                &["select_settings"],
                expected,
            );
        }
        for tier_consumer in ["a".into(), "a".repeat(128), "é".repeat(64)] {
            let request = parse_select_request_serving(&select_with(json!({"settings": {"project_id": "project-1", "tier_consumer": tier_consumer}})), &["select_settings"]).unwrap();
            assert_eq!(request.consumer, "flow");
            assert_eq!(
                request.settings,
                Some(
                    SelectSettings::new()
                        .with_project_id("project-1")
                        .with_tier_consumer(tier_consumer)
                )
            );
        }
        let request = parse_select_request_serving(
            &select_with(json!({"settings": {"project_id": null, "tier_consumer": null}})),
            &["select_settings"],
        )
        .unwrap();
        assert_eq!(request.settings, Some(SelectSettings::new()));
        assert_eq!(
            serde_json::to_value(request).unwrap()["settings"],
            json!({})
        );
    }

    #[test]
    fn answer_project_settings_is_present_exactly_for_a_project_lookup() {
        let plain = SelectRequest::new("flow", Requirements::iq(50));
        let project = plain
            .clone()
            .with_settings(SelectSettings::new().with_project_id("project-1"));
        assert_eq!(
            check_answer(&project, &answer()),
            Err(AnswerProblem::MissingProjectSettings)
        );
        let without_project = [
            plain.clone(),
            plain.clone().with_settings(SelectSettings::new()),
            plain.with_settings(SelectSettings::new().with_tier_consumer("another-consumer")),
        ];
        for request in &without_project {
            assert_eq!(check_answer(request, &answer()), Ok(()));
        }
        for (text, status) in [
            ("applied", ProjectSettings::Applied),
            ("none", ProjectSettings::None),
            ("unresolved", ProjectSettings::Unresolved),
            ("future", ProjectSettings::Other("future".into())),
        ] {
            let selected = answer().with_project_settings(status.clone());
            assert_eq!(check_answer(&project, &selected), Ok(()));
            assert_eq!(
                serde_json::to_value(&selected).unwrap()["project_settings"],
                text
            );
            assert_eq!(
                serde_json::from_value::<ProjectSettings>(json!(text)).unwrap(),
                status
            );
            for request in &without_project {
                assert_eq!(
                    check_answer(request, &selected),
                    Err(AnswerProblem::UnexpectedProjectSettings)
                );
            }
        }
    }

    #[test]
    fn settings_project_check_allows_no_stamp_project() {
        assert_eq!(
            SelectSettings::new()
                .with_project_id("project-1")
                .check_project_stamp(None),
            Ok(())
        );
    }

    #[test]
    fn settings_project_check_allows_no_requested_project() {
        assert_eq!(
            SelectSettings::new()
                .with_tier_consumer("another-consumer")
                .check_project_stamp(Some("project-1")),
            Ok(())
        );
    }

    #[test]
    fn settings_project_check_allows_exactly_matching_ids() {
        let settings = SelectSettings::new().with_project_id("opaque project / id");
        assert_eq!(
            settings.check_project_stamp(Some("opaque project / id")),
            Ok(())
        );
    }

    #[test]
    fn settings_project_check_refuses_a_different_stamp_by_field_path() {
        let settings = SelectSettings::new().with_project_id("project-1");
        for stamp in ["project-2", "PROJECT-1", "project-1 "] {
            let refusal = settings.check_project_stamp(Some(stamp)).unwrap_err();
            assert_eq!(refusal.code, RefusalCode::InvalidRequest);
            assert_eq!(field(&refusal), "settings.project_id");
        }
    }

    #[test]
    fn select_addition_builders_round_trip_with_their_groups() {
        let request = SelectRequest::new("flow", Requirements::iq(50))
            .with_pin_slot("primary")
            .with_pin_fallback(PinFallback::Route)
            .with_available_routes(vec!["example-provider/model-a".into()])
            .with_prefer_prompt_cache(true)
            .with_settings(
                SelectSettings::new()
                    .with_project_id("project-1")
                    .with_tier_consumer("another-consumer"),
            );
        let params = serde_json::to_value(&request).unwrap();
        assert_eq!(
            parse_select_request_serving(
                &params,
                &[
                    "select_pin_slot",
                    "select_available_routes",
                    "select_prefer_prompt_cache",
                    "select_settings"
                ]
            )
            .unwrap(),
            request
        );
        let selected = answer()
            .with_pin_outcome(PinOutcome::Pinned)
            .with_project_settings(ProjectSettings::Unresolved);
        assert_eq!(check_answer(&request, &selected), Ok(()));
        assert_eq!(
            serde_json::from_value::<SelectAnswer>(serde_json::to_value(&selected).unwrap())
                .unwrap(),
            selected
        );
    }

    #[test]
    fn legacy_select_bodies_round_trip_byte_for_byte_without_additions() {
        for body in [
            r#"{"consumer":"flow","requirements":{"iq":50}}"#,
            r#"{"service_tier":"fast","consumer":"flow","requirements":{"iq":50,"eq":30,"context_window":128000,"max_output_tokens":8000,"tools":"required","tool_capabilities":["code.read/v1"],"modalities":["image","pdf"],"structured_output":true},"pin":"example-provider/model-a:fast","exclude":["example-provider/model-b"],"exclude_providers":["other-provider"],"runners":["runner-one"],"affinity":"conversation-1","correlation_id":"work-1","task_ref":"task-1","concurrency_slots":[{"key":"runner-one","running":0,"limit":8,"queued":1}],"detail":true,"hold_ok":false,"hold_wait_max_s":0,"harness":"direct","prefer":["example-provider/model-a"],"pin_fallback":"route"}"#,
        ] {
            let request: SelectRequest = serde_json::from_str(body).unwrap();
            assert_eq!(
                serde_json::to_string(&request).unwrap().as_bytes(),
                body.as_bytes()
            );
            assert!(request.pin_slot.is_none());
            assert!(request.available_routes.is_none());
            assert!(request.prefer_prompt_cache.is_none());
            assert!(request.settings.is_none());
        }
        for body in [
            r#"{"decision_id":"decision-1","route":{"provider":"example-provider","model":"model-a"},"runner":{"module":"runner-one","model":{"provider":"example-provider","model":"model-a"}},"context_window":128000,"max_output_tokens":8000,"expires_at_ms":1000,"explanation":"Selected an adequate route."}"#,
            r#"{"service_tier_chosen":"fast","decision_id":"decision-1","route":{"provider":"example-provider","model":"model-a","variant":"fast"},"runner":{"module":"runner-one","model":{"provider":"example-provider","model":"model-a","variant":"fast"},"auth":{"pool":"example-pool","credential_id":"credential-1"},"harness":"direct"},"context_window":128000,"max_output_tokens":8000,"expires_at_ms":1000,"explanation":"Selected an adequate route.","detail":{"pin_reason":"cooldown"},"pin_outcome":"fell_back"}"#,
        ] {
            let selected: SelectAnswer = serde_json::from_str(body).unwrap();
            assert_eq!(
                serde_json::to_string(&selected).unwrap().as_bytes(),
                body.as_bytes()
            );
            assert!(selected.project_settings.is_none());
        }
    }
}
