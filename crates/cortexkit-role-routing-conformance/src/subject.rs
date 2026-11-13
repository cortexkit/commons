//! What a provider under test supplies: routes under the caller identities
//! and scopes the kit names, and hooks that set up the provider's
//! configuration before each check.

use async_trait::async_trait;
use cortexkit_role_routing::DemandDefaults;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

use crate::route::RoutingRoute;

/// The daemon's bind scope stamp, matching `(owner, ref, epoch)`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopeStamp {
    /// The attested principal owning the scope, not necessarily the caller.
    pub owner: String,
    pub scope_ref: String,
    pub scope_epoch: u64,
}

/// The caller identity and scope stamp a route carries.
///
/// `principal` is the module the route is opened as: a consumer module for
/// `route.select` and `route.report` (including a runner selecting its own
/// declared consumer), a runner module for the runner ops. The adapter opens
/// the route with that module's attested `reserved:<module>` identity.
/// `project_id`, when present, is the project the route is bound to; a
/// route without either project or scope is unscoped. Declaration admission
/// accepts the declaring module's `principal` or `scope.owner`, independently
/// of project attribution.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct RouteStamp {
    pub principal: String,
    pub project_id: Option<String>,
    pub scope: Option<ScopeStamp>,
}

impl RouteStamp {
    /// A route opened as `principal`, bound to no project.
    pub fn unscoped(principal: &str) -> Self {
        Self {
            principal: principal.to_owned(),
            project_id: None,
            scope: None,
        }
    }

    /// A route opened as `principal`, bound to `project_id`.
    pub fn project(principal: &str, project_id: &str) -> Self {
        Self {
            principal: principal.to_owned(),
            project_id: Some(project_id.to_owned()),
            scope: None,
        }
    }

    /// A route carrying a daemon-stamped scope under an independent caller.
    pub fn scoped(principal: &str, scope: ScopeStamp) -> Self {
        Self {
            principal: principal.to_owned(),
            project_id: None,
            scope: Some(scope),
        }
    }
}

/// Operator demand overrides. Unlike declarations, configuration may set `iq`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorRequirements {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub iq: Option<u8>,
    /// The demand fields that declarations may also default.
    #[serde(flatten)]
    pub defaults: DemandDefaults,
}

/// One consumer in the provider's configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsumerConfig {
    pub id: String,
    /// Answer this consumer only with routes priced per token.
    pub priced_only: bool,
    /// Operator demand values, which override request and declared values.
    pub requirements: OperatorRequirements,
}

/// A module's raw daemon catalog declaration slots. Ownership is trusted only
/// when the daemon nonce-verified the entry; the JSON never attests itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogModule {
    pub principal: String,
    pub nonce_verified: bool,
    pub role_declarations: BTreeMap<String, Value>,
}

/// Route transport facts used to intersect runner declarations with the catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RunnerRouteFacts {
    pub route_key: String,
    pub wire_api: String,
    pub auth_method: String,
}

impl RunnerRouteFacts {
    pub fn new(
        route_key: impl Into<String>,
        wire_api: impl Into<String>,
        auth_method: impl Into<String>,
    ) -> Self {
        Self {
            route_key: route_key.into(),
            wire_api: wire_api.into(),
            auth_method: auth_method.into(),
        }
    }
}

/// A model in the provider's catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogModel {
    pub provider: String,
    pub model: String,
    /// Models in the same family cannot occupy different seats of a panel.
    /// `None` means the family is unknown, and all such models count as one
    /// family.
    pub family: Option<String>,
    /// The model's ability on the provider's own 0-100 scale; a request's
    /// `iq` at or below it is met.
    pub iq: u8,
    pub context_window: u64,
    pub max_output_tokens: u64,
    /// The model accepts tool calls.
    pub tool_calls: bool,
    /// Input modalities the model accepts (`image`, `pdf`, `audio`).
    pub modalities: Vec<String>,
    pub structured_output: bool,
    /// Priced per token. A model that is not is paid from subscription
    /// quota, through accounts the provider allocates.
    pub priced_per_token: bool,
}

impl CatalogModel {
    /// The route key, `provider/model`.
    pub fn route_key(&self) -> String {
        format!("{}/{}", self.provider, self.model)
    }
}

/// A tool provider a runner can reach, and the tool capability tags it
/// serves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReachableToolProvider {
    /// The tool provider's module id. Routing never matches on it.
    pub module: String,
    /// The tool capability tags it serves, such as `code.read/v1`.
    pub capabilities: Vec<String>,
}

/// A runner module in the provider's catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogRunner {
    pub module: String,
    /// The route keys it runs.
    pub route_keys: Vec<String>,
    /// The tool providers it can reach, for every project.
    pub tool_providers: Vec<ReachableToolProvider>,
    /// Exact session programs from the runner's declaration, not route labels.
    pub harnesses: Option<Vec<String>>,
    pub default_harness: Option<String>,
    /// False excludes selection only; the runner may still allocate accounts.
    pub starts_sessions: bool,
}

/// A vault account the provider allocates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogAccount {
    pub provider: String,
    /// Catalog providers named by the vault at credential creation. Only
    /// `method == "gateway"` makes these borrowed-row sources for gateway routes.
    pub serves: Vec<String>,
    pub account_id: String,
    pub credential_id: String,
    pub method: String,
}

/// The models, runners and accounts the provider knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Catalog {
    /// In the order the provider prefers them when nothing else decides.
    pub models: Vec<CatalogModel>,
    /// In the order the provider prefers them when nothing else decides.
    pub runners: Vec<CatalogRunner>,
    pub accounts: Vec<CatalogAccount>,
}

/// How the operator routes one consumer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ConsumerMode {
    /// `mode: "demand"`: the next-cheapest adequate model, in catalog order
    /// when nothing else decides.
    Demand,
    /// `mode: "ladder"`: these entries, in order, each a route
    /// `provider/model`.
    Ladder { entries: Vec<String> },
}

/// The operator's per-provider hold settings, in seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HoldLimits {
    /// The longest the provider lets a runner hold.
    pub max_hold_s: u64,
    /// A blocked entry is held on only when its reset is at most this far
    /// away.
    pub hold_when_reset_within_s: u64,
}

impl HoldLimits {
    pub fn new(max_hold_s: u64, hold_when_reset_within_s: u64) -> Self {
        Self {
            max_hold_s,
            hold_when_reset_within_s,
        }
    }
}

/// A continuation credential's known account and eligible models. These facts
/// let checks detect cross-provider, cross-auth and cross-model offers without
/// trusting metadata returned by the router being tested.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ContinueCredential {
    pub account: CatalogAccount,
    pub models: Vec<String>,
}

impl ContinueCredential {
    pub fn new(account: CatalogAccount, models: Vec<String>) -> Self {
        Self { account, models }
    }
}

/// Relative catalog costs for one media kind and provider/model pair.
/// Smaller known cost ranks first. Values are fixture units, not billing amounts.
/// The legacy accuracy member is retained for source compatibility but no longer
/// supplies accuracy: send reference evaluations through `route.engine_eval`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EngineFacts {
    pub kind: crate::wire::select_engine::Kind,
    pub provider: String,
    pub model: String,
    pub accuracy: Option<u64>,
    pub cost: Option<u64>,
}

impl EngineFacts {
    pub fn new(
        kind: crate::wire::select_engine::Kind,
        provider: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            provider: provider.into(),
            model: model.into(),
            accuracy: None,
            cost: None,
        }
    }

    pub fn with_accuracy(mut self, accuracy: u64) -> Self {
        self.accuracy = Some(accuracy);
        self
    }

    pub fn with_cost(mut self, cost: u64) -> Self {
        self.cost = Some(cost);
        self
    }
}

/// Per-model facts supplied by the decision-input requirement fixture hook.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ModelInputFacts {
    pub route_key: String,
    /// The model's served per-request question limit, if stated.
    pub max_questions: Option<u64>,
    /// The catalog's stated input price, in USD per million tokens.
    pub input_per_mtok_usd: Option<serde_json::Number>,
}

impl ModelInputFacts {
    pub fn new(
        route_key: impl Into<String>,
        max_questions: Option<u64>,
        input_per_mtok_usd: Option<serde_json::Number>,
    ) -> Self {
        Self {
            route_key: route_key.into(),
            max_questions,
            input_per_mtok_usd,
        }
    }
}

/// A routing provider under test.
///
/// Before every check the kit calls [`reset`](Self::reset), then
/// [`configure_consumers`](Self::configure_consumers) and
/// [`configure_catalog`](Self::configure_catalog) with the kit's fixture
/// ([`crate::fixture`]); some checks then add a project pin or a cooldown.
/// The provider's own choice must be stable: the same request against the
/// same state answers the same route and runner.
#[async_trait]
pub trait RoutingSubject: Send + Sync {
    /// Test-only route kinds, catalog list rates and subscription value multipliers.
    /// A router declaring a capability group these facts feed (such as
    /// `select_kind` or `select_price`) must support them; no production
    /// configuration format is prescribed.
    async fn configure_selection_facts(&self, _facts: &SelectionFacts) -> Result<(), String> {
        Err("selection facts are not supported by this subject".into())
    }
    type Route: RoutingRoute;

    /// Return the provider to a fresh state: no consumers, catalog, pins,
    /// cooldowns, decisions, affinity, reports, admissions or observations.
    async fn reset(&self) -> Result<(), String>;

    /// Configure exactly these consumers.
    async fn configure_consumers(&self, consumers: &[ConsumerConfig]) -> Result<(), String>;

    /// Configure exactly this catalog. Every runner in it is an attested
    /// runner module (contract section 9: its route carries a principal the
    /// daemon verified, and the provider knows it as a runner); every other
    /// principal is not.
    async fn configure_catalog(&self, catalog: &Catalog) -> Result<(), String>;

    /// Configure the wire API and credential method for runner route matching.
    /// Unsupported adapters skip pair-specific checks, not provider-limit checks.
    /// Provider-only fixtures retain configured runnable keys with any mapping
    /// compatible with their runner-level pairs; only provider limits vary.
    async fn configure_runner_route_facts(
        &self,
        _facts: &[RunnerRouteFacts],
    ) -> Result<bool, String> {
        Ok(false)
    }

    /// Set or remove a catalog row's served `wire.api` endpoint fact, regardless
    /// of its provenance. Return false if the adapter cannot configure that fact.
    async fn configure_route_wire_api(
        &self,
        _route_key: &str,
        _endpoint: Option<&str>,
    ) -> Result<bool, String> {
        Ok(false)
    }

    /// Replace question-limit and input-price facts for the current catalog.
    /// Return false when this fixture hook is unavailable; the checks then
    /// report not applicable rather than passing without exercising a rule.
    async fn configure_model_input_facts(&self, facts: &[ModelInputFacts]) -> Result<bool, String> {
        let _ = facts;
        Ok(false)
    }

    /// Configure a panel class's counts. Its quality floor must be met by
    /// every model in the panel fixture. Required only with `select_panel`.
    async fn configure_panel_class(
        &self,
        _class: &str,
        _size: u8,
        _top: u8,
        _extra: u8,
    ) -> Result<(), String> {
        Err("configure_panel_class is not supported by this subject".into())
    }

    /// Map an operator-policy slot to a route key. Required only with
    /// `select_pin_slot`.
    async fn configure_pin_slot(&self, _slot: &str, _route_key: &str) -> Result<(), String> {
        Err("configure_pin_slot is not supported by this subject".into())
    }

    /// Install these credentials and their exact model eligibility, preserving
    /// existing admissions and decisions, with selection and failover enabled.
    /// Called only for `account_continue`; a declaring router must implement it.
    async fn configure_continue_credentials(
        &self,
        _credentials: &[ContinueCredential],
    ) -> Result<(), String> {
        Err("continuation credential fixtures are not supported by this subject".into())
    }

    /// Test-only plan knowledge for the configured account ids. Standard is
    /// always included; the list supplies non-standard inclusions. This hook
    /// does not prescribe where a router gets plans in production. Called only
    /// for the service_tier group; a declaring router must support the fixture.
    async fn configure_service_tiers(
        &self,
        _plans: &std::collections::BTreeMap<String, Vec<crate::wire::service_tier::ServiceTier>>,
    ) -> Result<(), String> {
        Err("service-tier plan fixtures are not supported by this subject".into())
    }

    /// Supply module entries through the daemon catalog, never request params.
    /// Older adapters fail declaration checks explicitly until they implement
    /// this hook; checks that use only the original catalog need no change.
    async fn configure_catalog_declarations(
        &self,
        modules: &[CatalogModule],
    ) -> Result<(), String> {
        if modules.is_empty() {
            Ok(())
        } else {
            Err("catalog declarations are not supported by this subject".into())
        }
    }

    /// Replace media cost fixtures, including facts for caller-local
    /// engines. Required only by the `select_engine` ranking checks.
    async fn configure_engine_facts(&self, _facts: &[EngineFacts]) -> Result<(), String> {
        Err("configure_engine_facts is not supported by this subject".into())
    }

    /// Advance the provider's clock by this duration and fire every timer due in
    /// the span before returning. Preserve decisions and admission replay state.
    /// Required only by the `select_engine` expiry checks.
    async fn advance_clock(&self, _ms: u64) -> Result<(), String> {
        Err("advance_clock is not supported by this subject".into())
    }

    /// An operator pin: within `project_id`, `consumer`'s selects that carry
    /// no `pin` of their own are answered with `pin` (`provider/model`).
    async fn pin_for_project(
        &self,
        project_id: &str,
        consumer: &str,
        pin: &str,
    ) -> Result<(), String>;

    /// Cool the route `route_key` (`provider/model`) down until `until_ms`
    /// (Unix milliseconds): it serves nothing before then.
    async fn cool_down(&self, route_key: &str, until_ms: u64) -> Result<(), String>;

    /// Route `consumer` by `mode`. Used only against a provider declaring
    /// `select_hold_control`; the default fails those checks.
    async fn configure_consumer_mode(
        &self,
        consumer: &str,
        mode: &ConsumerMode,
    ) -> Result<(), String> {
        let _ = (consumer, mode);
        Err("consumer modes are not supported by this subject".into())
    }

    /// Set the operator's `max_hold` and `hold_when_reset_within` for
    /// `provider`. Used only against a provider declaring
    /// `select_hold_control`; the default fails those checks.
    async fn configure_hold_limits(
        &self,
        provider: &str,
        limits: HoldLimits,
    ) -> Result<(), String> {
        let _ = (provider, limits);
        Err("hold limits are not supported by this subject".into())
    }

    /// Block the catalog account `account_id` by a window that ends at
    /// `reset_at_ms` (Unix milliseconds), or at an unknown time when
    /// `None`. A later call replaces the account's block. Used only against
    /// a provider declaring `select_hold_control`, `account_continue` or
    /// `account_status`; the default fails those checks.
    async fn block_account(
        &self,
        account_id: &str,
        reset_at_ms: Option<u64>,
    ) -> Result<(), String> {
        let _ = (account_id, reset_at_ms);
        Err("account blocks are not supported by this subject".into())
    }

    /// Open a route to the provider under `stamp`.
    async fn route(&self, stamp: &RouteStamp) -> Result<Self::Route, String>;
}

/// Facts joined by route key for the consumer/placement conformance cases.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct SelectionFacts {
    pub media: std::collections::BTreeMap<String, MediaRouteFacts>,
    pub kinds: std::collections::BTreeMap<String, String>,
    pub list_rates: std::collections::BTreeMap<String, crate::wire::price::RatesPerMtok>,
    pub multipliers: std::collections::BTreeMap<String, serde_json::Number>,
    pub estimates: std::collections::BTreeMap<String, serde_json::Number>,
}

/// Catalog facts for one media provider/model, independent of its declaration.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct MediaRouteFacts {
    pub grade: Option<u64>,
    pub price: Option<crate::wire::select_media::UnitPrice>,
    pub max_output_duration_ms: Option<u64>,
    pub aspect_ratios: Vec<String>,
    pub audio: Option<bool>,
    pub not_for: Vec<String>,
}

impl MediaRouteFacts {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SelectionFacts {
    pub fn new() -> Self {
        Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Reply, RouteFailure};

    struct LegacySubject;
    struct UnusedRoute;

    #[async_trait]
    impl RoutingRoute for UnusedRoute {
        async fn request(&self, _: &str, _: &Value) -> Result<Reply, RouteFailure> {
            unreachable!("default-hook tests never open a route")
        }
    }

    #[async_trait]
    impl RoutingSubject for LegacySubject {
        type Route = UnusedRoute;
        async fn reset(&self) -> Result<(), String> {
            unreachable!()
        }
        async fn configure_consumers(&self, _: &[ConsumerConfig]) -> Result<(), String> {
            unreachable!()
        }
        async fn configure_catalog(&self, _: &Catalog) -> Result<(), String> {
            unreachable!()
        }
        async fn pin_for_project(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
            unreachable!()
        }
        async fn cool_down(&self, _: &str, _: u64) -> Result<(), String> {
            unreachable!()
        }
        async fn route(&self, _: &RouteStamp) -> Result<UnusedRoute, String> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn routing_070_new_hooks_default_to_named_errors() {
        assert!(LegacySubject
            .configure_panel_class("audit", 3, 2, 1)
            .await
            .unwrap_err()
            .contains("configure_panel_class"));
        assert!(LegacySubject
            .configure_pin_slot("review", "provider/model")
            .await
            .unwrap_err()
            .contains("configure_pin_slot"));
    }
}
