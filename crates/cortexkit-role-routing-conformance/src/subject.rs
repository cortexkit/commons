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
/// `route.select` and `route.report`, a runner module for the runner ops.
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

/// A model in the provider's catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogModel {
    pub provider: String,
    pub model: String,
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

/// A subscription account the provider allocates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogAccount {
    pub provider: String,
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
    /// a provider declaring `select_hold_control`; the default fails those
    /// checks.
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
