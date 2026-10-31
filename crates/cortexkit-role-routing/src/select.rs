//! `route.select`: the request, its validator, and the answer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    capabilities::SELECT_HOLD_CONTROL,
    errors::{PinUnavailableReason, Refusal},
    members::Members,
};
/// Every requirement member this version defines, in contract order. A
/// `routing/v1` provider reads every one of them; `role.describe` lists them
/// in `requirement_fields`.
pub const REQUIREMENT_FIELDS: &[&str] = &[
    "iq",
    "eq",
    "context_window",
    "max_output_tokens",
    "tools",
    "tool_capabilities",
    "modalities",
    "structured_output",
];

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
    /// `pin_fallback: "route"` beside a pin: when the pin cannot be served,
    /// route normally instead of refusing `pin_unavailable`. Without it the
    /// pin still refuses and a second select for the same work stays
    /// forbidden. Any other value is `invalid_request` naming the field, and
    /// a `pin_fallback` without a pin is too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_fallback: Option<PinFallback>,
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
/// Whether the consumer is known and owned, and
/// whether anything can serve, is the provider's to answer after this.
pub fn parse_select_request_serving<C: AsRef<str>>(
    params: &Value,
    capabilities: &[C],
) -> Result<SelectRequest, Refusal> {
    let members = Members::of(params)?;
    let consumer = members.text("consumer", None)?;
    let service_tier = crate::service_tier::parse_request(members.object.get("service_tier"))
        .map_err(|reason| {
            let mut refusal = members.refuse("service_tier", reason);
            refusal
                .detail
                .insert("reason".into(), Value::String(reason.into()));
            refusal
        })?;
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
    if let Some(pin) = &pin {
        if RouteRef::parse_pin(pin).is_none() {
            return Err(Refusal::pin_unavailable(
                pin.clone(),
                PinUnavailableReason::Invalid,
                None,
            ));
        }
    }
    let exclude: Option<Vec<String>> = members.optional("exclude")?;
    if let Some(exclude) = &exclude {
        if let Some(bad) = exclude.iter().find(|key| !is_route_key(key)) {
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
        if pin.is_none() {
            return Err(members.refuse("pin_fallback", "pin_fallback without a pin is invalid"));
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
            if entry.contains(':') || !is_route_key(entry) {
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
    })
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
    Ok(Requirements {
        iq,
        eq,
        context_window: members.optional("context_window")?,
        max_output_tokens: members.optional("max_output_tokens")?,
        tools,
        tool_capabilities,
        modalities,
        structured_output: members.optional("structured_output")?,
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
        }
    }
}

/// Check an answer against the request it answers: the runner is one the
/// caller listed, the route is not one it excluded nor from a provider it
/// excluded, the explanation is within its bound, and a pinned request is
/// answered with a `pin_outcome` (with `pin_reason` in `detail` on
/// `fell_back`). A chosen tier must be recognised and may not upgrade the
/// request. This is stricter than record decoding, which retains unknown tiers.
pub fn check_answer(request: &SelectRequest, answer: &SelectAnswer) -> Result<(), AnswerProblem> {
    if (request.service_tier.is_some() && answer.service_tier_chosen.is_none())
        || answer.service_tier_chosen.as_deref().is_some_and(|tier| {
            crate::service_tier::ServiceTier::parse(tier)
                .is_none_or(|tier| tier > request.service_tier.unwrap_or_default())
        })
    {
        return Err(AnswerProblem::ServiceTier);
    }
    if let Some(runners) = &request.runners {
        if !runners.contains(&answer.runner.module) {
            return Err(AnswerProblem::RunnerNotListed(answer.runner.module.clone()));
        }
    }
    let key = answer.route.route_key();
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
    match (&request.pin, &answer.pin_outcome) {
        (Some(_), None) => return Err(AnswerProblem::MissingPinOutcome),
        (None, Some(_)) => return Err(AnswerProblem::UnexpectedPinOutcome),
        (Some(_), Some(outcome)) if *outcome == PinOutcome::FellBack => {
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
}
