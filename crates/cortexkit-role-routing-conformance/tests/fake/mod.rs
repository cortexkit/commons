//! A reference in-process routing provider, used ONLY to test the kit.
//!
//! Conformance runs against real providers. This fake exists so the kit's
//! own tests can show that each check passes a provider that keeps the
//! contract and fails one that breaks it. The property it models:
//!
//! > A provider that reads the caller and the project from the route, never
//! > the body; refuses unknown requirements and consumers by name; answers
//! > the first catalog model that meets the request (after a pin, a project
//! > pin, or an affinity key's still-healthy previous route) on the first
//! > runner that runs it, is in `runners`, and reaches a tool provider
//! > serving every required tag; carries `reset_at_ms` on every timed
//! > refusal; binds each decision to its principal and runner; and keeps
//! > reports, admissions and observations idempotent on their keys.
//!
//! Each field of [`Defects`] breaks exactly one of those rules.

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use cortexkit_role_routing_conformance::{
    wire::{
        account::{
            parse_account_for_request, Account, AccountForAnswer, AccountForRequest, AccountMode,
            AccountPin, AccountReason, AccountReasonCode, Admission,
        },
        capabilities::{
            SELECT_EXCLUDE_PROVIDERS, SELECT_HARNESS, SELECT_HOLD_CONTROL, SELECT_PREFER,
        },
        describe::{ConsumerEntry, RoleDescribe},
        errors::{NoRouteReason, PinUnavailableReason, Refusal, RefusalCode},
        observe::{parse_account_observe_request, AccountObserveAnswer, Observation, ObserveKind},
        ops, parse_consumer_declarations,
        report::{parse_report_request, ReportAnswer, ReportRequest, RESET_FENCE_CAP_MS},
        select::{
            parse_select_request_serving, PinFallback, PinOutcome, Requirements, RouteRef,
            RunnerRef, SelectAnswer, ToolsRequirement, REQUIREMENT_FIELDS,
        },
        wire, DemandDefaults, ROLE_DECLARATIONS_KEY,
    },
    Catalog, CatalogModel, CatalogModule, CatalogRunner, ConsumerConfig, ConsumerMode, HoldLimits,
    Reply, RouteFailure, RouteStamp, RoutingRoute, RoutingSubject,
};
use serde::Serialize;
use serde_json::Value;
use subc_protocol::ErrorBody;

/// How long a decision stays current.
const DECISION_TTL_MS: u64 = 600_000;

/// Deliberate contract breaks: each field makes the fake violate one rule a
/// check must catch.
#[derive(Clone, Copy, Debug, Default)]
pub struct Defects {
    /// Keeps the parent's account even when another eligible account exists.
    pub ignores_parent_preference: bool,
    /// Refuses rather than falling back to the sole eligible parent account.
    pub refuses_parent_fallback: bool,
    /// Turns model ineligibility into an account hold.
    pub holds_on_eligibility: bool,
    /// Refuses a well-formed pool that has no recorded block.
    pub refuses_unknown_pool: bool,
    /// Uses the wrong detail key for a malformed pool.
    pub malformed_pool_wrong_detail: bool,
    /// Omits pool from admission body equality.
    pub ignores_pool_body: bool,
    /// Omits the parent key from admission body equality.
    pub ignores_parent_body: bool,
    /// Omits the new report members from decision-id body equality.
    pub ignores_report_members: bool,
    /// role.describe states priced_only false for every consumer.
    pub describe_drops_priced_only: bool,
    /// A requirement the provider does not read is dropped, not refused.
    pub ignores_unknown_requirements: bool,
    /// The unknown-consumer check's probe is answered as if configured.
    pub answers_unknown_consumer: bool,
    /// `exclude` is ignored.
    pub ignores_exclude: bool,
    /// `runners` is ignored, so the answer may name a runner outside it.
    pub ignores_runners: bool,
    /// Timed refusals leave out `reset_at_ms`.
    pub drops_reset_at_ms: bool,
    /// A `project_id` member in the select body overrides the route's
    /// project.
    pub reads_scope_from_body: bool,
    /// A report on a decision is accepted from any principal.
    pub accepts_report_from_any_principal: bool,
    /// The same report body again is recorded anew, with a new answer.
    pub rerecords_repeated_report: bool,
    /// A changed outcome after the first replaces it instead of being
    /// refused report_conflict.
    pub overwrites_conflicting_report: bool,
    /// A failure report does not end the decision's affinity preference.
    pub affinity_survives_failure: bool,
    /// `expires_at_ms` is already in the past.
    pub expiry_in_past: bool,
    /// The same admission again is allocated anew, with a new answer.
    pub account_for_reallocates_on_replay: bool,
    /// A changed conversation key under a used admission_id is allocated
    /// instead of being refused admission_conflict.
    pub account_for_ignores_body_change: bool,
    /// A decision_id is honoured from any runner.
    pub account_for_ignores_decision_runner: bool,
    /// A decision_id is honoured for any provider and model.
    pub account_for_ignores_decision_model: bool,
    /// A repeated event_id is applied again and never answered duplicate.
    pub observe_never_duplicate: bool,
    /// Tool capability tags are matched against tool provider module names.
    pub tools_matched_by_module_name: bool,
    /// Serves a declared consumer without either ownership arm.
    pub serves_foreign_consumer: bool,
    /// Ignores a declaration's default when the request omits the demand.
    pub ignores_declared_default: bool,
    /// Applies declared demand after configured demand.
    pub declaration_overrides_config: bool,
    /// Checks the caller principal but ignores the stamped scope owner.
    pub admits_only_declaring_principal: bool,
    /// Trusts module declarations whose catalog entries were not nonce-verified.
    pub trusts_unverified_declarations: bool,
    /// Keeps the first verified owner instead of locking a conflicting name.
    pub first_declarer_wins_conflict: bool,
    /// Names a harness for a runner that declared none.
    pub invents_harness: bool,
    /// Leaves out the harness a runner declared.
    pub drops_harness: bool,
    pub ignores_harness_filter: bool,
    pub echoes_default_harness: bool,
    pub ignores_single_harness: bool,
    pub chooses_first_harness: bool,
    /// With no requested or default program, chooses the first of several declared programs.
    pub chooses_first_without_default: bool,
    pub selects_gateway: bool,
    pub accepts_undeclared_harness: bool,
    pub wrong_harness_refusal: bool,
    /// Moves past a blocked ladder entry even when its reset is within
    /// every limit.
    pub never_holds_entry: bool,
    /// Holds on a blocked entry whose reset is beyond
    /// hold_when_reset_within, as long as it is within the hold limit.
    pub ignores_hold_window: bool,
    /// Holds on a blocked entry whose reset is unknown.
    pub holds_on_unknown_reset: bool,
    /// Takes the hold limit from max_hold alone, ignoring the request's
    /// hold_wait_max_s.
    pub ignores_caller_cap: bool,
    /// Walks the ladder as if hold_ok were true.
    pub holds_despite_hold_ok_false: bool,
    /// With hold_ok false and nothing usable now, falls back to the first
    /// holdable entry in ladder order instead of the soonest.
    pub fallback_in_ladder_order: bool,
    /// accounts_blocked carries the first blocked entry's reset, not the
    /// earliest.
    pub accounts_blocked_reset_from_first_entry: bool,
    /// accounts_blocked carries a reset even when none is known.
    pub accounts_blocked_invents_reset: bool,
    /// A demand-mode consumer is answered a model whose accounts are all
    /// blocked when its reset is within the limits.
    pub holds_in_demand_mode: bool,
    /// account_for naming a decision ignores the cap its select recorded.
    pub account_for_ignores_recorded_cap: bool,
    /// A fallback answer is labelled pinned instead of fell_back.
    pub pin_fallback_labels_pinned: bool,
    /// A fell_back answer omits pin_reason from detail.
    pub pin_fallback_omits_pin_reason: bool,
    /// Ignores prefer and always answers the natural first candidate.
    pub ignores_prefer: bool,
    /// Treats prefer as a pin: answers the first listed route even when it
    /// is not a candidate.
    pub admits_prefer: bool,
    /// Honours prefer among adequate routes, but when no route meets the
    /// request answers the first listed route instead of refusing.
    pub admits_prefer_when_nothing_meets: bool,
    /// Accepts prefer without declaring select_prefer.
    pub accepts_undeclared_prefer: bool,
    /// A fenced report blocks every account and route, not just the
    /// decision's.
    pub fences_wide: bool,
    /// A past reset still fences.
    pub past_reset_fences: bool,
    /// A past reset is refused as invalid_request.
    pub past_reset_refused: bool,
    /// A reset beyond 24 hours is used uncapped.
    pub reset_not_capped: bool,
    /// Accepts a report usage with a bad count or unknown member.
    pub accepts_bad_usage: bool,
    /// Omits usage from decision-id body equality.
    pub ignores_usage_body: bool,
    /// Declares select_exclude_providers but ignores `exclude_providers`.
    pub ignores_exclude_providers: bool,
    /// Honours `exclude_providers` while another route is adequate, but when
    /// every adequate route is from an excluded provider answers the first
    /// of them instead of refusing no_route.
    pub answers_excluded_provider_when_nothing_else: bool,
    /// Accepts exclude_providers without declaring select_exclude_providers.
    pub accepts_undeclared_exclude_providers: bool,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

struct Decision {
    principal: String,
    route: RouteRef,
    runner: String,
    /// The select request's hold_wait_max_s.
    hold_cap: Option<u64>,
}

struct AffinityEntry {
    decision_id: String,
    /// A failure report on the decision ended the preference.
    ended: bool,
}

/// The pin a conversation holds on an account.
struct ConversationPin {
    id: String,
    generation: u64,
    account: usize,
}

#[derive(Default)]
struct State {
    held_credentials: Vec<String>,
    ineligible: Vec<(String, Option<String>)>,
    consumers: Vec<ConsumerConfig>,
    catalog: Option<Catalog>,
    declarations: BTreeMap<String, (Option<String>, DemandDefaults)>,
    /// (project, consumer) to pin.
    project_pins: HashMap<(String, String), String>,
    /// Route key to the end of its cooldown.
    cooldowns: HashMap<String, u64>,
    decisions: HashMap<String, Decision>,
    /// (principal, consumer, affinity key) to its last decision.
    affinity: HashMap<(String, String, String), AffinityEntry>,
    reports: HashMap<String, (ReportRequest, ReportAnswer)>,
    /// (runner, admission id) to its first body and answer.
    admissions: HashMap<(String, String), (AccountForRequest, AccountForAnswer)>,
    conversations: HashMap<String, ConversationPin>,
    /// (runner, event id) to its observation.
    observations: HashMap<(String, String), Observation>,
    /// Consumer to its ladder entries; a consumer not listed is routed by
    /// demand.
    ladders: HashMap<String, Vec<String>>,
    /// Model provider to (max_hold, hold_when_reset_within), in seconds.
    hold_limits: HashMap<String, (u64, u64)>,
    /// Account id to the end of its blocking window; `None` when unknown.
    blocks: HashMap<String, Option<u64>>,
    /// (route key, account id) to the end of a report fence.
    fences: HashMap<(String, String), u64>,
    /// Decision id to the account index its account_for named.
    decision_accounts: HashMap<String, usize>,
    counter: u64,
}

/// The contract's default max_hold and hold_when_reset_within, in seconds.
const DEFAULT_HOLD_S: u64 = 900;

impl State {
    fn next(&mut self, prefix: &str) -> String {
        self.counter += 1;
        format!("{prefix}-{}", self.counter)
    }

    fn hold_limits(&self, provider: &str) -> (u64, u64) {
        self.hold_limits
            .get(provider)
            .copied()
            .unwrap_or((DEFAULT_HOLD_S, DEFAULT_HOLD_S))
    }

    /// Whether the account is fenced for the route at `now`.
    fn fenced(&self, route_key: &str, account_id: &str, now: u64) -> Option<u64> {
        self.fences
            .get(&(route_key.to_owned(), account_id.to_owned()))
            .copied()
            .filter(|reset| *reset > now)
    }

    /// Whether every account of `provider` is blocked at `now`, and if so
    /// the soonest known end of their windows. `None` when an account is
    /// usable, or the provider has no accounts.
    fn blocked(&self, catalog: &Catalog, provider: &str, now: u64) -> Option<Option<u64>> {
        let mut accounts = catalog
            .accounts
            .iter()
            .filter(|account| account.provider == provider)
            .peekable();
        accounts.peek()?;
        let mut soonest: Option<u64> = None;
        for account in accounts {
            match self.blocks.get(&account.account_id) {
                Some(Some(reset)) if *reset > now => {
                    soonest = Some(soonest.map_or(*reset, |s| s.min(*reset)));
                }
                Some(None) => {}
                _ => return None,
            }
        }
        Some(soonest)
    }
}

pub struct FakeSubject {
    state: Arc<Mutex<State>>,
    defects: Defects,
    /// Declares select_hold_control.
    hold_control: bool,
    harness_control: bool,
    prefer_control: bool,
    exclude_providers_control: bool,
    /// Reads a request's iq relative to the strongest routable model instead
    /// of on the catalog's own 0-100 scale. The contract leaves that mapping
    /// to the provider, so both readings are correct.
    relative_iq: bool,
}

impl FakeSubject {
    pub fn new(defects: Defects) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            defects,
            hold_control: true,
            harness_control: true,
            prefer_control: true,
            exclude_providers_control: true,
            relative_iq: false,
        }
    }

    /// A fake that does not declare select_hold_control.
    pub fn without_hold_control(defects: Defects) -> Self {
        Self {
            hold_control: false,
            ..Self::new(defects)
        }
    }

    pub fn without_harness_control(defects: Defects) -> Self {
        Self {
            harness_control: false,
            ..Self::new(defects)
        }
    }

    pub fn without_prefer_control(defects: Defects) -> Self {
        Self {
            prefer_control: false,
            ..Self::new(defects)
        }
    }

    /// A fake that does not declare select_exclude_providers.
    pub fn without_exclude_providers_control(defects: Defects) -> Self {
        Self {
            exclude_providers_control: false,
            ..Self::new(defects)
        }
    }

    /// A fake that maps a request's iq relative to its strongest routable
    /// model: iq 100 asks for that model's ability, so it is always met.
    pub fn with_relative_iq(defects: Defects) -> Self {
        Self {
            relative_iq: true,
            ..Self::new(defects)
        }
    }
}

#[async_trait]
impl RoutingSubject for FakeSubject {
    type Route = FakeRoute;

    async fn reset(&self) -> Result<(), String> {
        *self.state.lock().unwrap() = State::default();
        Ok(())
    }

    async fn configure_consumers(&self, consumers: &[ConsumerConfig]) -> Result<(), String> {
        self.state.lock().unwrap().consumers = consumers.to_vec();
        Ok(())
    }

    async fn configure_catalog(&self, catalog: &Catalog) -> Result<(), String> {
        self.state.lock().unwrap().catalog = Some(catalog.clone());
        Ok(())
    }

    async fn configure_catalog_declarations(
        &self,
        modules: &[CatalogModule],
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        state.declarations.clear();
        for module in modules
            .iter()
            .filter(|module| module.nonce_verified || self.defects.trusts_unverified_declarations)
        {
            let Some(raw) = module.role_declarations.get(ROLE_DECLARATIONS_KEY) else {
                continue;
            };
            let Ok(declaration) = parse_consumer_declarations(raw) else {
                continue;
            };
            for consumer in declaration.consumers {
                let entry = state
                    .declarations
                    .entry(consumer.name)
                    .or_insert_with(|| (Some(module.principal.clone()), consumer.defaults));
                if entry.0.as_deref() != Some(module.principal.as_str())
                    && !self.defects.first_declarer_wins_conflict
                {
                    entry.0 = None;
                }
            }
        }
        Ok(())
    }

    async fn pin_for_project(
        &self,
        project_id: &str,
        consumer: &str,
        pin: &str,
    ) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .project_pins
            .insert((project_id.to_owned(), consumer.to_owned()), pin.to_owned());
        Ok(())
    }

    async fn cool_down(&self, route_key: &str, until_ms: u64) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .cooldowns
            .insert(route_key.to_owned(), until_ms);
        Ok(())
    }

    async fn configure_consumer_mode(
        &self,
        consumer: &str,
        mode: &ConsumerMode,
    ) -> Result<(), String> {
        let mut state = self.state.lock().unwrap();
        match mode {
            ConsumerMode::Ladder { entries } => {
                state.ladders.insert(consumer.to_owned(), entries.clone());
            }
            _ => {
                state.ladders.remove(consumer);
            }
        }
        Ok(())
    }

    async fn configure_hold_limits(
        &self,
        provider: &str,
        limits: HoldLimits,
    ) -> Result<(), String> {
        self.state.lock().unwrap().hold_limits.insert(
            provider.to_owned(),
            (limits.max_hold_s, limits.hold_when_reset_within_s),
        );
        Ok(())
    }

    async fn block_account(
        &self,
        account_id: &str,
        reset_at_ms: Option<u64>,
    ) -> Result<(), String> {
        self.state
            .lock()
            .unwrap()
            .blocks
            .insert(account_id.to_owned(), reset_at_ms);
        Ok(())
    }

    async fn route(&self, stamp: &RouteStamp) -> Result<FakeRoute, String> {
        Ok(FakeRoute {
            state: Arc::clone(&self.state),
            defects: self.defects,
            hold_control: self.hold_control,
            harness_control: self.harness_control,
            prefer_control: self.prefer_control,
            exclude_providers_control: self.exclude_providers_control,
            relative_iq: self.relative_iq,
            stamp: stamp.clone(),
        })
    }
}

/// A route to the fake, carrying the caller and scope it was opened under.
pub struct FakeRoute {
    state: Arc<Mutex<State>>,
    defects: Defects,
    hold_control: bool,
    harness_control: bool,
    prefer_control: bool,
    exclude_providers_control: bool,
    relative_iq: bool,
    stamp: RouteStamp,
}

#[async_trait]
impl RoutingRoute for FakeRoute {
    async fn request(&self, method: &str, params: &Value) -> Result<Reply, RouteFailure> {
        let mut state = self.state.lock().unwrap();
        let answered = match method {
            ops::ROLE_DESCRIBE => self.describe(&state),
            ops::ROUTE_SELECT => self.select(&mut state, params),
            ops::ROUTE_REPORT => self.report(&mut state, params),
            ops::ROUTE_ACCOUNT_FOR => self.account_for(&mut state, params),
            ops::ROUTE_ACCOUNT_OBSERVE => self.observe(&mut state, params),
            other => Err(Refusal::invalid_request("method", format!("no op {other}"))),
        };
        Ok(match answered {
            Ok(answer) => {
                Reply::Response(String::from_utf8(wire::reply_body(&answer).unwrap()).unwrap())
            }
            Err(refusal) => Reply::Error(
                ErrorBody::new(refusal.code.as_str(), refusal.message)
                    .with_detail(Value::Object(refusal.detail)),
            ),
        })
    }
}

fn to_value<T: Serialize>(answer: T) -> Result<Value, Refusal> {
    Ok(serde_json::to_value(answer).unwrap())
}

/// Whether `model` meets `requirements`. The request's iq is read on a scale
/// whose 100 is `iq_ceiling` on the catalog's own scale: 100 reads it as an
/// absolute value, the strongest routable model's iq reads it relative to
/// that model.
fn meets(requirements: &Requirements, model: &CatalogModel, iq_ceiling: u8) -> bool {
    u32::from(requirements.iq) * u32::from(iq_ceiling) <= u32::from(model.iq) * 100
        && requirements
            .context_window
            .is_none_or(|floor| floor <= model.context_window)
        && requirements
            .max_output_tokens
            .is_none_or(|floor| floor <= model.max_output_tokens)
        && (requirements.tools != Some(ToolsRequirement::Required) || model.tool_calls)
        && requirements
            .modalities
            .iter()
            .flatten()
            .all(|modality| model.modalities.iter().any(|m| m == modality.as_str()))
        && (requirements.structured_output != Some(true) || model.structured_output)
}

impl FakeRoute {
    fn principal(&self) -> &str {
        &self.stamp.principal
    }

    fn catalog(state: &State) -> Catalog {
        state.catalog.clone().unwrap_or(Catalog {
            models: vec![],
            runners: vec![],
            accounts: vec![],
        })
    }

    fn describe(&self, state: &State) -> Result<Value, Refusal> {
        let mut consumers: Vec<_> = state
            .consumers
            .iter()
            .map(|consumer| ConsumerEntry {
                id: consumer.id.clone(),
                priced_only: consumer.priced_only && !self.defects.describe_drops_priced_only,
            })
            .collect();
        consumers.retain(|consumer| {
            state
                .declarations
                .get(&consumer.id)
                .is_none_or(|(owner, _)| owner.is_some())
        });
        for (id, (owner, _)) in &state.declarations {
            if owner.is_some() && !consumers.iter().any(|consumer| &consumer.id == id) {
                consumers.push(ConsumerEntry {
                    id: id.clone(),
                    priced_only: false,
                });
            }
        }
        let mut describe = RoleDescribe::new(consumers, true);
        if self.harness_control {
            describe = describe.with_capability(SELECT_HARNESS);
        }
        if self.prefer_control {
            describe = describe.with_capability(SELECT_PREFER);
        }
        if self.exclude_providers_control {
            describe = describe.with_capability(SELECT_EXCLUDE_PROVIDERS);
        }
        to_value(if self.hold_control {
            describe.with_capability(SELECT_HOLD_CONTROL)
        } else {
            describe
        })
    }

    fn reaches(&self, runner: &CatalogRunner, tags: &[String]) -> bool {
        tags.iter().all(|tag| {
            runner.tool_providers.iter().any(|provider| {
                if self.defects.tools_matched_by_module_name {
                    &provider.module == tag
                } else {
                    provider.capabilities.contains(tag)
                }
            })
        })
    }

    fn select(&self, state: &mut State, params: &Value) -> Result<Value, Refusal> {
        let defects = self.defects;
        let mut params = params.clone();
        if defects.ignores_unknown_requirements {
            if let Some(Value::Object(requirements)) = params.get_mut("requirements") {
                requirements.retain(|key, _| REQUIREMENT_FIELDS.contains(&key.as_str()));
            }
        }
        let mut capabilities = Vec::new();
        if self.hold_control {
            capabilities.push(SELECT_HOLD_CONTROL);
        }
        if self.harness_control || defects.accepts_undeclared_harness {
            capabilities.push(SELECT_HARNESS);
        }
        if self.prefer_control || defects.accepts_undeclared_prefer {
            capabilities.push(SELECT_PREFER);
        }
        if self.exclude_providers_control || defects.accepts_undeclared_exclude_providers {
            capabilities.push(SELECT_EXCLUDE_PROVIDERS);
        }
        let mut request = parse_select_request_serving(&params, &capabilities)?;
        let declaration = state.declarations.get(&request.consumer);
        if let Some((owner, _)) = declaration {
            let scope_owner = self.stamp.scope.as_ref().map(|scope| scope.owner.as_str());
            let owned = owner.as_deref().is_some_and(|owner| {
                self.principal() == owner
                    || (!defects.admits_only_declaring_principal && scope_owner == Some(owner))
            });
            if owner.is_none() || (!defects.serves_foreign_consumer && !owned) {
                return Err(Refusal::consumer_not_owned(request.consumer.clone()));
            }
        }
        let consumer = match state.consumers.iter().find(|c| c.id == request.consumer) {
            Some(consumer) => consumer.clone(),
            None if declaration.is_some()
                || (defects.answers_unknown_consumer
                    && request.consumer
                        == cortexkit_role_routing_conformance::fixture::UNCONFIGURED) =>
            {
                ConsumerConfig {
                    id: request.consumer.clone(),
                    priced_only: false,
                    requirements: Default::default(),
                }
            }
            None => return Err(Refusal::unknown_consumer(request.consumer.clone())),
        };
        let declared = declaration
            .map(|(_, defaults)| serde_json::to_value(defaults).unwrap())
            .unwrap_or_else(|| serde_json::json!({}));
        let configured = serde_json::to_value(&consumer.requirements).unwrap();
        let mut demands = if defects.ignores_declared_default {
            serde_json::Map::new()
        } else {
            declared.as_object().unwrap().clone()
        };
        demands.extend(
            serde_json::to_value(&request.requirements)
                .unwrap()
                .as_object()
                .unwrap()
                .clone(),
        );
        demands.extend(configured.as_object().unwrap().clone());
        if defects.declaration_overrides_config && !configured.as_object().unwrap().is_empty() {
            demands.extend(declared.as_object().unwrap().clone());
        }
        request.requirements = serde_json::from_value(Value::Object(demands)).unwrap();
        let catalog = Self::catalog(state);
        if let Some(harness) = &request.harness {
            if !defects.ignores_harness_filter
                && !catalog.runners.iter().any(|runner| {
                    runner.starts_sessions
                        && runner
                            .harnesses
                            .as_ref()
                            .is_some_and(|values| values.contains(harness))
                })
            {
                let mut refusal = Refusal::no_route(
                    if defects.wrong_harness_refusal {
                        NoRouteReason::MissingCapability
                    } else {
                        NoRouteReason::HarnessUnavailable
                    },
                    None,
                );
                refusal.message = format!("no runner hosts harness {harness}");
                return Err(refusal);
            }
        }
        let project = match params.get("project_id").and_then(Value::as_str) {
            Some(project) if defects.reads_scope_from_body => Some(project.to_owned()),
            _ => self.stamp.project_id.clone(),
        };
        let exclude = match &request.exclude {
            Some(exclude) if !defects.ignores_exclude => exclude.clone(),
            _ => vec![],
        };
        let excluded_providers = match &request.exclude_providers {
            Some(providers) if !defects.ignores_exclude_providers => providers.clone(),
            _ => vec![],
        };
        // A route is ruled out by its key in `exclude` or its provider in
        // `exclude_providers`, at the same point of every walk, so the two
        // lead to the same refusal.
        let excluded = |model: &CatalogModel| {
            exclude.contains(&model.route_key()) || excluded_providers.contains(&model.provider)
        };
        let runners = request.runners.clone().filter(|_| !defects.ignores_runners);
        let tags = request
            .requirements
            .tool_capabilities
            .clone()
            .unwrap_or_default();
        let now = now_ms();
        let reset = |until: u64| (!defects.drops_reset_at_ms).then_some(until);
        let cooled = |key: &str| {
            state
                .cooldowns
                .get(key)
                .copied()
                .filter(|until| *until > now)
        };
        // The strongest model some session-starting runner runs.
        let strongest = catalog
            .models
            .iter()
            .filter(|model| {
                let key = model.route_key();
                catalog
                    .runners
                    .iter()
                    .any(|runner| runner.starts_sessions && runner.route_keys.contains(&key))
            })
            .map(|model| model.iq)
            .max()
            .unwrap_or(0);
        let iq_ceiling = if self.relative_iq { strongest } else { 100 };
        let eligible = |model: &CatalogModel| {
            (!consumer.priced_only || model.priced_per_token)
                && meets(&request.requirements, model, iq_ceiling)
        };
        let runner_for = |key: &str| {
            catalog
                .runners
                .iter()
                .find(|runner| {
                    runner.route_keys.iter().any(|k| k == key)
                        && (runner.starts_sessions || defects.selects_gateway)
                        && (defects.ignores_harness_filter
                            || request.harness.as_ref().is_none_or(|harness| {
                                runner
                                    .harnesses
                                    .as_ref()
                                    .is_some_and(|values| values.contains(harness))
                            }))
                        && runners
                            .as_ref()
                            .is_none_or(|listed| listed.contains(&runner.module))
                        && self.reaches(runner, &tags)
                })
                .map(|runner| runner.module.clone())
        };
        let blocked = |model: &CatalogModel| {
            if model.priced_per_token {
                None
            } else {
                state.blocked(&catalog, &model.provider, now)
            }
        };

        let pin = request.pin.clone().or_else(|| {
            project.and_then(|project| {
                state
                    .project_pins
                    .get(&(project, consumer.id.clone()))
                    .cloned()
            })
        });
        let mut fell_back_reason: Option<PinUnavailableReason> = None;
        let (model, variant, runner) = if let Some(pin) = pin {
            #[allow(clippy::type_complexity)]
            let attempt: Result<
                (CatalogModel, Option<String>, String),
                (PinUnavailableReason, Option<u64>),
            > = (|| {
                let Some(route) = RouteRef::parse_pin(&pin) else {
                    return Err((PinUnavailableReason::Invalid, None));
                };
                if !catalog.models.iter().any(|m| m.provider == route.provider) {
                    return Err((PinUnavailableReason::UnknownProvider, None));
                }
                let key = route.route_key();
                let Some(model) = catalog.models.iter().find(|m| m.route_key() == key) else {
                    return Err((PinUnavailableReason::UnknownModel, None));
                };
                if excluded(model) {
                    return Err((PinUnavailableReason::Excluded, None));
                }
                if let Some(until) = cooled(&key) {
                    return Err((PinUnavailableReason::Cooldown, reset(until)));
                }
                if !eligible(model) {
                    return Err((PinUnavailableReason::NotAvailable, None));
                }
                let Some(runner) = runner_for(&key) else {
                    return Err((PinUnavailableReason::NotAvailable, None));
                };
                Ok((model.clone(), route.variant, runner))
            })();
            match attempt {
                Ok((model, variant, runner)) => (model, variant, runner),
                Err((reason, _reset_at))
                    if request.pin.is_some()
                        && request.pin_fallback == Some(PinFallback::Route) =>
                {
                    fell_back_reason = Some(reason.clone());
                    let mut fallback: Option<(CatalogModel, Option<String>, String)> = None;
                    for model in catalog.models.iter().filter(|m| eligible(m)) {
                        let key = model.route_key();
                        if excluded(model) {
                            continue;
                        }
                        if cooled(&key).is_some() {
                            continue;
                        }
                        if let Some(runner) = runner_for(&key) {
                            fallback = Some((model.clone(), None, runner));
                            break;
                        }
                    }
                    match fallback {
                        Some((model, variant, runner)) => (model, variant, runner),
                        None => return Err(Refusal::no_route(NoRouteReason::AllInadequate, None)),
                    }
                }
                Err((reason, reset_at)) => {
                    return Err(Refusal::pin_unavailable(pin.clone(), reason, reset_at));
                }
            }
        } else {
            let ladder = state.ladders.get(&consumer.id).cloned();
            let ordered: Vec<CatalogModel> = match &ladder {
                Some(entries) => entries
                    .iter()
                    .filter_map(|entry| catalog.models.iter().find(|m| &m.route_key() == entry))
                    .cloned()
                    .collect(),
                None => catalog.models.clone(),
            };
            // Every entry that would serve, in order, with whether all its
            // accounts are blocked and, if so, its reset.
            let mut entries = Vec::new();
            let mut earliest: Option<u64> = None;
            let mut unreachable = false;
            // Routes ruled out only by their provider, kept for the defect
            // that answers one when nothing else is left.
            let mut provider_excluded = Vec::new();
            for model in ordered.iter().filter(|m| eligible(m)) {
                let key = model.route_key();
                if excluded(model) {
                    if defects.answers_excluded_provider_when_nothing_else
                        && !exclude.contains(&key)
                    {
                        if let Some(runner) = runner_for(&key) {
                            provider_excluded.push((model.clone(), runner));
                        }
                    }
                    continue;
                }
                let Some(runner) = runner_for(&key) else {
                    unreachable = true;
                    continue;
                };
                if let Some(until) = cooled(&key) {
                    earliest = Some(earliest.map_or(until, |e: u64| e.min(until)));
                    continue;
                }
                entries.push((model.clone(), runner, blocked(model)));
            }
            // Whether a blocked entry with this reset may be held on: steps
            // 2 and 3 of the contract's hold decision.
            let holdable = |model: &CatalogModel, reset: Option<u64>| {
                let (max_hold, within) = state.hold_limits(&model.provider);
                let cap = request
                    .hold_wait_max_s
                    .filter(|_| !defects.ignores_caller_cap);
                let limit = cap.map_or(max_hold, |cap| cap.min(max_hold));
                let Some(reset) = reset else {
                    return defects.holds_on_unknown_reset;
                };
                let wait = reset.saturating_sub(now);
                (wait <= within * 1000 || defects.ignores_hold_window) && wait <= limit * 1000
            };
            let chosen = if ladder.is_some() {
                let hold_ok = request.hold_ok() || defects.holds_despite_hold_ok_false;
                let walked = entries.iter().find(|(model, _, blocked)| match blocked {
                    None => true,
                    Some(reset) => hold_ok && holdable(model, *reset) && !defects.never_holds_entry,
                });
                walked
                    .or_else(|| {
                        if hold_ok {
                            return None;
                        }
                        // Nothing usable now: hold on the entry that clears
                        // soonest within the limits.
                        let mut fallback = entries
                            .iter()
                            .filter_map(|entry| match entry.2 {
                                Some(Some(reset)) if holdable(&entry.0, Some(reset)) => {
                                    Some((reset, entry))
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>();
                        if !defects.fallback_in_ladder_order {
                            fallback.sort_by_key(|(reset, _)| *reset);
                        }
                        fallback.first().map(|(_, entry)| *entry)
                    })
                    .map(|(model, runner, _)| (model.clone(), runner.clone()))
            } else {
                let candidates: Vec<_> = entries
                    .iter()
                    .filter(|(model, _, blocked)| match blocked {
                        None => true,
                        Some(reset) => defects.holds_in_demand_mode && holdable(model, *reset),
                    })
                    .collect();
                if !defects.ignores_prefer {
                    if let Some(prefer) = &request.prefer {
                        if defects.admits_prefer
                            || (defects.admits_prefer_when_nothing_meets && candidates.is_empty())
                        {
                            if let Some(first) = prefer.first() {
                                if let Some(model) =
                                    catalog.models.iter().find(|m| &m.route_key() == first)
                                {
                                    if let Some(runner) = runner_for(first) {
                                        let decision_id = state.next("decision");
                                        let route = RouteRef {
                                            provider: model.provider.clone(),
                                            model: model.model.clone(),
                                            variant: None,
                                        };
                                        state.decisions.insert(
                                            decision_id.clone(),
                                            Decision {
                                                principal: self.principal().to_owned(),
                                                route: route.clone(),
                                                runner: runner.clone(),
                                                hold_cap: request.hold_wait_max_s,
                                            },
                                        );
                                        return to_value(SelectAnswer::new(
                                            decision_id,
                                            route.clone(),
                                            RunnerRef::new(
                                                runner.clone(),
                                                RouteRef {
                                                    provider: model.provider.clone(),
                                                    model: model.model.clone(),
                                                    variant: None,
                                                },
                                            ),
                                            model.context_window,
                                            model.max_output_tokens,
                                            if defects.expiry_in_past {
                                                now - 1
                                            } else {
                                                now + DECISION_TTL_MS
                                            },
                                            format!(
                                                "{} meets the request on {runner}.",
                                                model.route_key()
                                            ),
                                        ));
                                    }
                                }
                            }
                        }
                        for wanted in prefer {
                            if let Some((model, runner, _)) = candidates
                                .iter()
                                .find(|(model, _, _)| &model.route_key() == wanted)
                            {
                                let decision_id = state.next("decision");
                                let route = RouteRef {
                                    provider: model.provider.clone(),
                                    model: model.model.clone(),
                                    variant: None,
                                };
                                state.decisions.insert(
                                    decision_id.clone(),
                                    Decision {
                                        principal: self.principal().to_owned(),
                                        route: route.clone(),
                                        runner: runner.clone(),
                                        hold_cap: request.hold_wait_max_s,
                                    },
                                );
                                if let Some(affinity) = &request.affinity {
                                    state.affinity.insert(
                                        (
                                            self.principal().to_owned(),
                                            consumer.id.clone(),
                                            affinity.clone(),
                                        ),
                                        AffinityEntry {
                                            decision_id: decision_id.clone(),
                                            ended: false,
                                        },
                                    );
                                }
                                let expires_at_ms = if defects.expiry_in_past {
                                    now - 1
                                } else {
                                    now + DECISION_TTL_MS
                                };
                                let declared = catalog
                                    .runners
                                    .iter()
                                    .find(|candidate| candidate.module == *runner)
                                    .and_then(|candidate| {
                                        request
                                            .harness
                                            .clone()
                                            .filter(|_| !defects.echoes_default_harness)
                                            .or_else(|| {
                                                if defects.chooses_first_harness {
                                                    candidate
                                                        .harnesses
                                                        .as_ref()
                                                        .and_then(|values| values.first().cloned())
                                                } else {
                                                    candidate.default_harness.clone()
                                                }
                                            })
                                            .or_else(|| {
                                                candidate
                                                    .harnesses
                                                    .as_ref()
                                                    .filter(|values| {
                                                        (values.len() == 1
                                                            && !defects.ignores_single_harness)
                                                            || (defects
                                                                .chooses_first_without_default
                                                                && request.harness.is_none()
                                                                && candidate
                                                                    .default_harness
                                                                    .is_none()
                                                                && values.len() > 1)
                                                    })
                                                    .map(|values| values[0].clone())
                                            })
                                    });
                                let harness = match declared {
                                    Some(_) if defects.drops_harness => None,
                                    None if defects.invents_harness => {
                                        Some("default-harness".to_owned())
                                    }
                                    declared => declared,
                                };
                                let mut runner_ref = RunnerRef::new(runner.clone(), route.clone());
                                if let Some(harness) = harness {
                                    runner_ref = runner_ref.with_harness(harness);
                                }
                                let mut answer = SelectAnswer::new(
                                    decision_id,
                                    route.clone(),
                                    runner_ref,
                                    model.context_window,
                                    model.max_output_tokens,
                                    expires_at_ms,
                                    format!("{} meets the request on {runner}.", route.route_key()),
                                );
                                if request.detail == Some(true) {
                                    answer =
                                        answer.with_detail(serde_json::json!({"order": "catalog"}));
                                }
                                if request.pin.is_some() {
                                    let outcome = if fell_back_reason.is_some() {
                                        PinOutcome::FellBack
                                    } else {
                                        PinOutcome::Pinned
                                    };
                                    if defects.pin_fallback_labels_pinned
                                        && outcome == PinOutcome::FellBack
                                    {
                                        answer = answer.with_pin_outcome(PinOutcome::Pinned);
                                    } else {
                                        answer = answer.with_pin_outcome(outcome);
                                    }
                                    if answer.pin_outcome == Some(PinOutcome::FellBack)
                                        && !defects.pin_fallback_omits_pin_reason
                                    {
                                        let reason = fell_back_reason
                                            .clone()
                                            .map(|reason| reason.as_str().to_owned())
                                            .unwrap_or_else(|| "not_available".to_owned());
                                        let mut detail = answer
                                            .detail
                                            .clone()
                                            .and_then(|detail| detail.as_object().cloned())
                                            .unwrap_or_default();
                                        detail
                                            .insert("pin_reason".to_owned(), Value::String(reason));
                                        answer = answer.with_detail(Value::Object(detail));
                                    }
                                }
                                return to_value(answer);
                            }
                        }
                    }
                }
                // The affinity key's previous route wins over the catalog
                // order while it is still a candidate: it meets the request
                // and is not cooling down. A failure report on it ends the
                // preference.
                let preferred = request.affinity.as_ref().and_then(|affinity| {
                    let entry = state.affinity.get(&(
                        self.principal().to_owned(),
                        consumer.id.clone(),
                        affinity.clone(),
                    ))?;
                    if entry.ended && !defects.affinity_survives_failure {
                        return None;
                    }
                    let previous = state.decisions.get(&entry.decision_id)?.route.route_key();
                    candidates
                        .iter()
                        .position(|(model, _, _)| model.route_key() == previous)
                });
                candidates
                    .get(preferred.unwrap_or(0))
                    .map(|(model, runner, _)| (model.clone(), runner.clone()))
            };
            let chosen = chosen.or_else(|| provider_excluded.first().cloned());
            match chosen {
                Some((model, runner)) => (model, None, runner),
                None => {
                    let resets: Vec<Option<u64>> =
                        entries.iter().filter_map(|entry| entry.2).collect();
                    if !resets.is_empty() {
                        let known = if defects.accounts_blocked_reset_from_first_entry {
                            resets[0]
                        } else {
                            resets.iter().flatten().min().copied()
                        };
                        let known = known.or_else(|| {
                            defects
                                .accounts_blocked_invents_reset
                                .then_some(now + 60_000)
                        });
                        return Err(Refusal::no_route(
                            NoRouteReason::AccountsBlocked,
                            known.and_then(reset),
                        ));
                    }
                    return Err(match earliest {
                        Some(until) => {
                            Refusal::no_route(NoRouteReason::QuotaOrCooldown, reset(until))
                        }
                        None if unreachable => {
                            Refusal::no_route(NoRouteReason::MissingCapability, None)
                        }
                        None => Refusal::no_route(NoRouteReason::AllInadequate, None),
                    });
                }
            }
        };

        let decision_id = state.next("decision");
        let route = RouteRef {
            provider: model.provider.clone(),
            model: model.model.clone(),
            variant,
        };
        state.decisions.insert(
            decision_id.clone(),
            Decision {
                principal: self.principal().to_owned(),
                route: route.clone(),
                runner: runner.clone(),
                hold_cap: request.hold_wait_max_s,
            },
        );
        if let Some(affinity) = &request.affinity {
            state.affinity.insert(
                (
                    self.principal().to_owned(),
                    consumer.id.clone(),
                    affinity.clone(),
                ),
                AffinityEntry {
                    decision_id: decision_id.clone(),
                    ended: false,
                },
            );
        }
        let expires_at_ms = if defects.expiry_in_past {
            now - 1
        } else {
            now + DECISION_TTL_MS
        };
        let declared = catalog
            .runners
            .iter()
            .find(|candidate| candidate.module == runner)
            .and_then(|candidate| {
                request
                    .harness
                    .clone()
                    .filter(|_| !defects.echoes_default_harness)
                    .or_else(|| {
                        if defects.chooses_first_harness {
                            candidate
                                .harnesses
                                .as_ref()
                                .and_then(|values| values.first().cloned())
                        } else {
                            candidate.default_harness.clone()
                        }
                    })
                    .or_else(|| {
                        candidate
                            .harnesses
                            .as_ref()
                            .filter(|values| {
                                (values.len() == 1 && !defects.ignores_single_harness)
                                    || (defects.chooses_first_without_default
                                        && request.harness.is_none()
                                        && candidate.default_harness.is_none()
                                        && values.len() > 1)
                            })
                            .map(|values| values[0].clone())
                    })
            });
        let harness = match declared {
            Some(_) if defects.drops_harness => None,
            None if defects.invents_harness => Some("default-harness".to_owned()),
            declared => declared,
        };
        let mut runner_ref = RunnerRef::new(runner.clone(), route.clone());
        if let Some(harness) = harness {
            runner_ref = runner_ref.with_harness(harness);
        }
        let mut answer = SelectAnswer::new(
            decision_id,
            route.clone(),
            runner_ref,
            model.context_window,
            model.max_output_tokens,
            expires_at_ms,
            format!("{} meets the request on {runner}.", route.route_key()),
        );
        if request.detail == Some(true) {
            answer = answer.with_detail(serde_json::json!({"order": "catalog"}));
        }
        // Pinned answers carry how the pin was served; unpinned carry none.
        // `fell_back_reason` is set only when the pin failed and fallback routed.
        if request.pin.is_some() {
            let outcome = if fell_back_reason.is_some() {
                PinOutcome::FellBack
            } else {
                PinOutcome::Pinned
            };
            if defects.pin_fallback_labels_pinned && outcome == PinOutcome::FellBack {
                answer = answer.with_pin_outcome(PinOutcome::Pinned);
            } else {
                answer = answer.with_pin_outcome(outcome);
            }
            if answer.pin_outcome == Some(PinOutcome::FellBack)
                && !defects.pin_fallback_omits_pin_reason
            {
                let reason = fell_back_reason
                    .clone()
                    .map(|reason| reason.as_str().to_owned())
                    .unwrap_or_else(|| "not_available".to_owned());
                let mut detail = answer
                    .detail
                    .clone()
                    .and_then(|detail| detail.as_object().cloned())
                    .unwrap_or_default();
                detail.insert("pin_reason".to_owned(), Value::String(reason));
                answer = answer.with_detail(Value::Object(detail));
            }
        }
        to_value(answer)
    }

    fn report(&self, state: &mut State, params: &Value) -> Result<Value, Refusal> {
        let defects = self.defects;
        let request = match parse_report_request(params) {
            Ok(request) => request,
            Err(refusal)
                if defects.accepts_bad_usage
                    && refusal.detail.get("field") == Some(&Value::String("usage".into())) =>
            {
                let mut stripped = params.clone();
                stripped
                    .as_object_mut()
                    .map(|object| object.remove("usage"));
                parse_report_request(&stripped)?
            }
            Err(refusal) => return Err(refusal),
        };
        let decision_route = {
            let Some(decision) = state.decisions.get(&request.decision_id) else {
                return Err(Refusal::unknown_decision());
            };
            if decision.principal != self.principal() && !defects.accepts_report_from_any_principal
            {
                return Err(Refusal::unknown_decision());
            }
            decision.route.clone()
        };
        if let Some((first, answer)) = state.reports.get(&request.decision_id) {
            let mut compared = request.clone();
            if defects.ignores_report_members {
                compared.pool.clone_from(&first.pool);
                compared.eligibility.clone_from(&first.eligibility);
                compared.request_input_tokens = first.request_input_tokens;
                compared.usage.clone_from(&first.usage);
            }
            if defects.ignores_usage_body {
                compared.usage.clone_from(&first.usage);
            }
            if *first == compared {
                if !defects.rerecords_repeated_report {
                    return to_value(answer);
                }
            } else if !defects.overwrites_conflicting_report || first.outcome == request.outcome {
                return Err(Refusal::new(
                    RefusalCode::ReportConflict,
                    "a different report is recorded for this decision",
                ));
            }
        }
        if let (Some(eligibility), Some(credential)) =
            (&request.eligibility, &request.credential_id)
        {
            let model = (eligibility.scope
                == cortexkit_role_routing_conformance::wire::report::EligibilityScope::Model)
                .then(|| decision_route.model.clone());
            state.ineligible.push((credential.clone(), model));
            if defects.holds_on_eligibility {
                state.held_credentials.push(credential.clone());
            }
        }
        let answer = ReportAnswer {
            decision_id: request.decision_id.clone(),
            report_id: state.next("report"),
            outcome: request.outcome.clone(),
        };
        if request.outcome.is_failure() {
            for entry in state.affinity.values_mut() {
                if entry.decision_id == request.decision_id {
                    entry.ended = true;
                }
            }
        }
        // A failed report with a reset fences only the decision's route on
        // its account. Only `failed` fences; other failures just end affinity.
        if request.outcome == cortexkit_role_routing_conformance::wire::report::Outcome::Failed {
            if let Some(reset) = request.reset_at_ms {
                let now = now_ms();
                if reset <= now {
                    if defects.past_reset_refused {
                        return Err(Refusal::invalid_request(
                            "reset_at_ms",
                            "a past reset is still recorded, never refused: this fake refuses it",
                        ));
                    }
                    if defects.past_reset_fences {
                        let catalog = Self::catalog(state);
                        let route_key = decision_route.route_key();
                        let account_id = state
                            .decision_accounts
                            .get(&request.decision_id)
                            .and_then(|index| catalog.accounts.get(*index))
                            .map(|account| account.account_id.clone())
                            .or_else(|| {
                                request.credential_id.as_ref().and_then(|credential| {
                                    catalog
                                        .accounts
                                        .iter()
                                        .find(|account| &account.credential_id == credential)
                                        .map(|account| account.account_id.clone())
                                })
                            });
                        if let Some(account_id) = account_id {
                            state.fences.insert((route_key, account_id), now + 600_000);
                        }
                    }
                } else {
                    let capped = if defects.reset_not_capped {
                        reset
                    } else {
                        reset.min(now + RESET_FENCE_CAP_MS)
                    };
                    let catalog = Self::catalog(state);
                    if defects.fences_wide {
                        for model in &catalog.models {
                            for account in catalog
                                .accounts
                                .iter()
                                .filter(|account| account.provider == model.provider)
                            {
                                state.fences.insert(
                                    (model.route_key(), account.account_id.clone()),
                                    capped,
                                );
                            }
                        }
                    } else {
                        let route_key = decision_route.route_key();
                        let account_id = state
                            .decision_accounts
                            .get(&request.decision_id)
                            .and_then(|index| catalog.accounts.get(*index))
                            .map(|account| account.account_id.clone())
                            .or_else(|| {
                                request.credential_id.as_ref().and_then(|credential| {
                                    catalog
                                        .accounts
                                        .iter()
                                        .find(|account| &account.credential_id == credential)
                                        .map(|account| account.account_id.clone())
                                })
                            });
                        if let Some(account_id) = account_id {
                            state.fences.insert((route_key, account_id), capped);
                        }
                    }
                }
            }
        }
        state
            .reports
            .insert(request.decision_id.clone(), (request, answer.clone()));
        to_value(answer)
    }

    fn is_runner(&self, state: &State) -> bool {
        Self::catalog(state)
            .runners
            .iter()
            .any(|runner| runner.module == self.principal())
    }

    fn account_for(&self, state: &mut State, params: &Value) -> Result<Value, Refusal> {
        let defects = self.defects;
        if !self.is_runner(state) {
            return Err(Refusal::new(
                RefusalCode::AccountNotAuthorized,
                "only runner modules call the account ops",
            ));
        }
        let request = parse_account_for_request(params).map_err(|mut refusal| {
            if defects.malformed_pool_wrong_detail
                && refusal.detail.get("field") == Some(&Value::String("pool".into()))
            {
                refusal.detail.remove("field");
                refusal
                    .detail
                    .insert("member".into(), Value::String("pool".into()));
            }
            refusal
        })?;
        let key = (self.principal().to_owned(), request.admission_id.clone());
        if let Some((first, answer)) = state.admissions.get(&key) {
            let mut compared = request.clone();
            if defects.ignores_pool_body {
                compared.pool.clone_from(&first.pool);
            }
            if defects.ignores_parent_body {
                compared
                    .parent_conversation_key
                    .clone_from(&first.parent_conversation_key);
            }
            if first.same_body(&compared) {
                if !defects.account_for_reallocates_on_replay {
                    return to_value(answer);
                }
            } else if !defects.account_for_ignores_body_change
                || first.conversation_key == request.conversation_key
            {
                return Err(Refusal::new(
                    RefusalCode::AdmissionConflict,
                    "the admission id was used with a different body",
                ));
            }
        }
        if defects.refuses_unknown_pool && request.pool.is_some() {
            return Err(Refusal::invalid_request("pool", "pool is not registered"));
        }
        if state.held_credentials.iter().any(|credential| {
            request.candidates.as_ref().is_some_and(|candidates| {
                candidates
                    .accounts
                    .iter()
                    .any(|account| &account.credential_id == credential)
            })
        }) {
            return Err(Refusal::account_hold(now_ms() + 60_000));
        }
        if let Some(decision_id) = &request.decision_id {
            let Some(decision) = state.decisions.get(decision_id) else {
                return Err(Refusal::unknown_decision());
            };
            if decision.runner != self.principal() && !defects.account_for_ignores_decision_runner {
                return Err(Refusal::unknown_decision());
            }
            let same_model = decision.route.provider == request.provider
                && decision.route.model == request.model;
            if !same_model && !defects.account_for_ignores_decision_model {
                return Err(Refusal::unknown_decision());
            }
        }
        let catalog = Self::catalog(state);
        let now = now_ms();
        if let Some(reset) = state.blocked(&catalog, &request.provider, now) {
            // Every account is blocked: hold within the smallest of
            // max_hold, the runner's own cap and the cap the decision's
            // select recorded; beyond it, refuse with the reset.
            let (max_hold, _) = state.hold_limits(&request.provider);
            let recorded = request
                .decision_id
                .as_ref()
                .and_then(|id| state.decisions.get(id))
                .and_then(|decision| decision.hold_cap)
                .filter(|_| !defects.account_for_ignores_recorded_cap);
            let own = request.hold_wait_max_s.unwrap_or(
                cortexkit_role_routing_conformance::wire::account::DEFAULT_HOLD_WAIT_MAX_S,
            );
            let limit = recorded.map_or(own, |cap| cap.min(own)).min(max_hold);
            return Err(match reset {
                Some(reset) if reset.saturating_sub(now) <= limit * 1000 => {
                    Refusal::account_hold(reset)
                }
                reset => Refusal::account_pool_exhausted(reset),
            });
        }
        let usable: Vec<usize> = catalog
            .accounts
            .iter()
            .enumerate()
            .filter(|(_, account)| account.provider == request.provider)
            .filter(|(_, account)| {
                !state.ineligible.iter().any(|(credential, model)| {
                    credential == &account.credential_id
                        && model.as_ref().is_none_or(|model| model == &request.model)
                })
            })
            .filter(|(_, account)| {
                request.candidates.as_ref().is_none_or(|candidates| {
                    candidates
                        .accounts
                        .iter()
                        .any(|c| c.account_id == account.account_id)
                })
            })
            .map(|(index, _)| index)
            .collect();
        let route_key = format!("{}/{}", request.provider, request.model);
        let mut unfenced = Vec::new();
        let mut fence_resets = Vec::new();
        for index in usable {
            let account_id = &catalog.accounts[index].account_id;
            if let Some(reset) = state.fenced(&route_key, account_id, now) {
                fence_resets.push(reset);
            } else {
                unfenced.push(index);
            }
        }
        if unfenced.is_empty() && !fence_resets.is_empty() {
            let earliest = fence_resets.into_iter().min().unwrap();
            return Err(Refusal::account_pool_exhausted(Some(earliest)));
        }
        let usable = unfenced;
        if usable.is_empty() {
            return Err(Refusal::new(
                RefusalCode::AuthRequired,
                "no usable account of this provider is signed in",
            ));
        }
        let requested = request
            .requested_account
            .as_ref()
            .and_then(|requested| requested.account_id.as_ref())
            .and_then(|id| {
                usable
                    .iter()
                    .copied()
                    .find(|index| &catalog.accounts[*index].account_id == id)
            });
        // The conversation's current pin: (id, generation, account index).
        let current: Option<(String, u64, usize)> = state
            .conversations
            .get(&request.conversation_key)
            .map(|pin| (pin.id.clone(), pin.generation, pin.account));
        let mode = request.mode();
        if mode != AccountMode::Admit {
            let expected = request
                .expected_pin
                .as_ref()
                .expect("checked by the parser");
            match &current {
                Some((id, generation, _))
                    if *id == expected.id && *generation == expected.generation => {}
                _ => {
                    return Err(Refusal::new(
                        RefusalCode::PinGenerationConflict,
                        "the pin is not at the expected generation",
                    ))
                }
            }
        }
        let parent = request
            .parent_conversation_key
            .as_ref()
            .and_then(|key| state.conversations.get(key))
            .map(|pin| pin.account);
        let away = parent.and_then(|parent| usable.iter().copied().find(|index| *index != parent));
        if parent.is_some() && away.is_none() && defects.refuses_parent_fallback {
            return Err(Refusal::new(
                RefusalCode::AccountSelectionUnavailable,
                "no account away from parent",
            ));
        }
        let (account, code) = match (&mode, &current) {
            (AccountMode::Reassign, Some((_, _, held))) => (
                usable
                    .iter()
                    .copied()
                    .find(|index| index != held)
                    .unwrap_or(usable[0]),
                AccountReasonCode::Reassigned,
            ),
            (AccountMode::Renew, Some((_, _, held))) => (*held, AccountReasonCode::Renewed),
            (_, Some((_, _, held))) if usable.contains(held) && requested.is_none() => {
                (*held, AccountReasonCode::Affinity)
            }
            _ => match requested {
                Some(index) => (index, AccountReasonCode::RequestedAccount),
                None => (
                    if defects.ignores_parent_preference {
                        usable[0]
                    } else {
                        away.unwrap_or(usable[0])
                    },
                    AccountReasonCode::Headroom,
                ),
            },
        };
        let code = if request.decision_id.is_some() {
            AccountReasonCode::Decision
        } else {
            code
        };
        let (pin_id, generation) = match (current, &mode) {
            (Some((id, generation, _)), AccountMode::Admit) => (id, generation),
            (Some((id, generation, _)), _) => (id, generation + 1),
            (None, _) => (state.next("pin"), 1),
        };
        state.conversations.insert(
            request.conversation_key.clone(),
            ConversationPin {
                id: pin_id.clone(),
                generation,
                account,
            },
        );
        let decision_id = match &request.decision_id {
            Some(decision_id) => decision_id.clone(),
            None => state.next("allocation"),
        };
        if request.decision_id.is_some() {
            state.decision_accounts.insert(decision_id.clone(), account);
        }
        let chosen = &catalog.accounts[account];
        let answer = AccountForAnswer {
            admission: Admission {
                id: request.admission_id.clone(),
                state: "admitted".to_owned(),
            },
            account: Account {
                provider: chosen.provider.clone(),
                account_id: chosen.account_id.clone(),
                credential_id: chosen.credential_id.clone(),
                method: chosen.method.clone(),
            },
            pin: AccountPin {
                id: pin_id,
                generation,
                affinity_expires_at_ms: now + 3_600_000,
                admission_valid_until_ms: now + 600_000,
                lease_valid_until_ms: now + 300_000,
                mode,
            },
            reason: AccountReason {
                code,
                decision_id,
                policy_version: "fake-1".to_owned(),
                quota_fetched_at_ms: None,
                quota_age_band: "unknown".to_owned(),
            },
        };
        state.admissions.insert(key, (request, answer.clone()));
        to_value(answer)
    }

    fn observe(&self, state: &mut State, params: &Value) -> Result<Value, Refusal> {
        if !self.is_runner(state) {
            return Err(Refusal::new(
                RefusalCode::AccountNotAuthorized,
                "only runner modules call the account ops",
            ));
        }
        let request = parse_account_observe_request(params)?;
        let key = (self.principal().to_owned(), request.event_id.clone());
        if let Some(first) = state.observations.get(&key) {
            if !self.defects.observe_never_duplicate {
                return to_value(AccountObserveAnswer {
                    observation: Observation {
                        id: first.id.clone(),
                        duplicate: true,
                        bound: first.bound,
                        effects: vec![],
                    },
                });
            }
        }
        let bound = request.admission_id.as_ref().is_some_and(|admission| {
            state
                .admissions
                .contains_key(&(self.principal().to_owned(), admission.clone()))
        });
        let effects = if request.kind == ObserveKind::AccountLimitExhausted {
            vec!["account_fenced".to_owned()]
        } else {
            vec![]
        };
        let observation = Observation {
            id: state.next("observation"),
            duplicate: false,
            bound,
            effects,
        };
        state.observations.insert(key, observation.clone());
        to_value(AccountObserveAnswer { observation })
    }
}
