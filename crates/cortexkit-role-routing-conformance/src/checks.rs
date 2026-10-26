//! The checks, each run against the live provider through its routes,
//! from a freshly reset and configured provider.

use std::time::{SystemTime, UNIX_EPOCH};

use cortexkit_role_routing::{
    account::AccountForAnswer,
    capabilities::{self, SELECT_HARNESS, SELECT_HOLD_CONTROL},
    describe::{check_describe, RoleDescribe},
    errors::{
        NoRouteDetail, NoRouteReason, PinUnavailableDetail, PinUnavailableReason, Refusal,
        RefusalCode,
    },
    observe::AccountObserveAnswer,
    ops,
    report::ReportAnswer,
    select::{check_answer, parse_select_request_serving, SelectAnswer},
    wire,
};
use serde::de::DeserializeOwned;
use serde_json::{json, Map, Value};

use crate::{
    fixture::{self, CONSUMER, HOLD_FIRST, HOLD_PRICED, HOLD_SECOND, IQ, SUMMARY},
    route::{Reply, RoutingRoute},
    subject::{
        CatalogModule, ConsumerConfig, ConsumerMode, HoldLimits, OperatorRequirements, RouteStamp,
        RoutingSubject, ScopeStamp,
    },
};

type Check = Result<(), String>;

/// How a check that did not fail ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ending {
    Passed,
    /// The check covers an optional group the provider does not declare.
    NotApplicable(String),
}

/// The checks of the `select_hold_control` group's rules (contract section
/// 5.4). Each is not applicable to a provider that does not declare it.
const HOLD_CHECKS: &[&str] = &[
    "hold_answers_blocked_entry_within_limits",
    "hold_moves_when_reset_beyond_window",
    "hold_moves_when_reset_unknown",
    "hold_moves_beyond_caller_cap",
    "hold_ok_false_moves_to_usable_entry",
    "hold_ok_false_falls_back_to_soonest_entry",
    "hold_nothing_within_limit_accounts_blocked",
    "hold_demand_mode_skips_blocked_model",
    "hold_account_for_applies_recorded_cap",
];

/// The operator hold settings the hold checks start from, in seconds: the
/// contract's defaults.
const MAX_HOLD_S: u64 = 900;
const HOLD_WHEN_RESET_WITHIN_S: u64 = 900;

/// A decision id no provider has made.
const NEVER_MADE: &str = "decision-never-made";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// The select most checks send: the summary consumer at the fixture's
/// demand.
fn base() -> Value {
    json!({"consumer": SUMMARY, "requirements": {"iq": IQ}})
}

/// `params` with the top-level member `key` set to `value`.
fn with(mut params: Value, key: &str, value: Value) -> Value {
    params[key] = value;
    params
}

/// A fixture route key other than `not`.
fn another_route_key(not: &str) -> Result<String, String> {
    fixture::catalog()
        .models
        .iter()
        .map(|model| model.route_key())
        .find(|key| key != not)
        .ok_or_else(|| "the fixture has a single model".to_owned())
}

/// The fixture runner other than `not`.
fn another_runner(not: &str) -> Result<String, String> {
    match not {
        fixture::RUNNER_ONE => Ok(fixture::RUNNER_TWO.to_owned()),
        fixture::RUNNER_TWO => Ok(fixture::RUNNER_ONE.to_owned()),
        other => Err(format!(
            "the answer names runner {other:?}, which the catalog does not list"
        )),
    }
}

/// Send `op` with `params` and read the reply: the decoded answer, or the
/// refusal. A broken route, a reply that is not `{"result": <answer>}`, or a
/// refusal detail that is not an object fails the check.
async fn call<R: RoutingRoute, T: DeserializeOwned>(
    route: &R,
    op: &str,
    params: &Value,
) -> Result<Result<T, Refusal>, String> {
    let reply = route
        .request(op, params)
        .await
        .map_err(|failure| format!("{op}: the route failed: {}", failure.message))?;
    match reply {
        Reply::Response(body) => wire::read_reply::<T>(body.as_bytes())
            .map(Ok)
            .map_err(|problem| format!("{op}: {problem}")),
        Reply::Error(body) => {
            let detail = match body.detail {
                None | Some(Value::Null) => Map::new(),
                Some(Value::Object(detail)) => detail,
                Some(other) => {
                    return Err(format!(
                        "{op}: the {} refusal's detail is not an object: {other}",
                        body.code
                    ))
                }
            };
            Ok(Err(Refusal {
                code: RefusalCode::parse(&body.code),
                message: body.message,
                detail,
            }))
        }
    }
}

/// Send `op` and require an answer.
async fn answer<R: RoutingRoute, T: DeserializeOwned>(
    route: &R,
    op: &str,
    params: &Value,
    what: &str,
) -> Result<T, String> {
    call(route, op, params)
        .await?
        .map_err(|refusal| format!("{what} was refused {}: {}", refusal.code, refusal.message))
}

/// Send `op` and require a refusal with `code`.
async fn refused<R: RoutingRoute>(
    route: &R,
    op: &str,
    params: &Value,
    code: RefusalCode,
    what: &str,
) -> Result<Refusal, String> {
    match call::<R, Value>(route, op, params).await? {
        Ok(answer) => Err(format!(
            "{what} was answered ({answer}), not refused {code}"
        )),
        Err(refusal) if refusal.code == code => Ok(refusal),
        Err(refusal) => Err(format!(
            "{what} was refused {}, not {code}: {}",
            refusal.code, refusal.message
        )),
    }
}

/// Send `route.select` and require an answer that keeps the rules a caller
/// can check alone (runners, exclude, explanation length).
async fn select<R: RoutingRoute>(
    route: &R,
    params: &Value,
    what: &str,
) -> Result<SelectAnswer, String> {
    let answer: SelectAnswer = answer(route, ops::ROUTE_SELECT, params, what).await?;
    // The kit sends a group's members only to a provider declaring the
    // group, so the request is read here as every group would read it.
    let request = parse_select_request_serving(params, capabilities::KNOWN)
        .map_err(|refusal| format!("the kit sent an invalid select: {refusal}"))?;
    check_answer(&request, &answer).map_err(|problem| format!("{what}: {problem}"))?;
    Ok(answer)
}

/// Two refusals that must not tell a caller apart a decision it may not see
/// from one that does not exist.
fn same_refusal(seen: &Refusal, never_made: &Refusal, what: &str) -> Check {
    if seen.code != never_made.code || seen.detail != never_made.detail {
        return Err(format!(
            "{what} was refused {} {:?}, but a decision that never existed is refused {} {:?}: the refusal reveals that the decision exists",
            seen.code, seen.detail, never_made.code, never_made.detail
        ));
    }
    Ok(())
}

/// The kit's state for one run.
pub struct Suite<'a, S: RoutingSubject> {
    subject: &'a S,
}

impl<'a, S: RoutingSubject> Suite<'a, S> {
    pub fn new(subject: &'a S) -> Self {
        Self { subject }
    }

    /// Run the check named `name` from a fresh provider.
    pub async fn run(&self, name: &str) -> Result<Ending, String> {
        self.fresh().await?;
        if HOLD_CHECKS.contains(&name) {
            return self.hold_check(name).await;
        }
        if matches!(
            name,
            "requested_harness_is_hosted_and_echoed" | "undeclared_harness_group_refuses_member"
        ) {
            let route = self.route_as(CONSUMER).await?;
            let describe: RoleDescribe =
                answer(&route, ops::ROLE_DESCRIBE, &json!({}), "role.describe").await?;
            let declared = describe.serves(SELECT_HARNESS);
            if declared == (name == "undeclared_harness_group_refuses_member") {
                return Ok(Ending::NotApplicable(format!(
                    "role.describe {} {SELECT_HARNESS}",
                    if declared {
                        "declares"
                    } else {
                        "does not declare"
                    }
                )));
            }
        }
        self.ungated(name).await.map(|()| Ending::Passed)
    }

    /// A check every provider is held to.
    async fn ungated(&self, name: &str) -> Check {
        match name {
            "describe_lists_consumers_and_fields" => {
                self.describe_lists_consumers_and_fields().await
            }
            "unknown_requirement_refused_by_name" => {
                self.unknown_requirement_refused_by_name().await
            }
            "unknown_consumer_refused" => self.unknown_consumer_refused().await,
            "declared_consumer_owner_uses_default" => {
                self.declared_consumer_owner_uses_default().await
            }
            "declared_consumer_scope_owner_admitted" => {
                self.declared_consumer_scope_owner_admitted().await
            }
            "declared_consumer_non_owner_refused" => {
                self.declared_consumer_non_owner_refused().await
            }
            "declared_consumer_unverified_entry_ignored" => {
                self.declared_consumer_unverified_entry_ignored().await
            }
            "declared_consumer_conflict_refused" => self.declared_consumer_conflict_refused().await,
            "operator_config_overrides_declared_default" => {
                self.operator_config_overrides_declared_default().await
            }
            "exclude_honoured" => self.exclude_honoured().await,
            "runners_honoured" => self.runners_honoured().await,
            "timed_refusals_carry_reset_at_ms" => self.timed_refusals_carry_reset_at_ms().await,
            "scope_comes_from_route_stamp" => self.scope_comes_from_route_stamp().await,
            "report_from_other_principal_unknown_decision" => {
                self.report_from_other_principal_unknown_decision().await
            }
            "repeated_report_returns_first_answer" => {
                self.repeated_report_returns_first_answer().await
            }
            "different_report_is_report_conflict" => {
                self.different_report_is_report_conflict().await
            }
            "affinity_holds_until_failure_report" => {
                self.affinity_holds_until_failure_report().await
            }
            "answer_carries_limits_and_future_expiry" => {
                self.answer_carries_limits_and_future_expiry().await
            }
            "account_for_idempotent_on_admission_id" => {
                self.account_for_idempotent_on_admission_id().await
            }
            "account_for_different_body_admission_conflict" => {
                self.account_for_different_body_admission_conflict().await
            }
            "account_for_foreign_decision_unknown" => {
                self.account_for_foreign_decision_unknown().await
            }
            "account_observe_idempotent_per_event_id" => {
                self.account_observe_idempotent_per_event_id().await
            }
            "tool_capabilities_matched_by_tag" => self.tool_capabilities_matched_by_tag().await,
            "parent_account_preference" => self.parent_account_preference(false).await,
            "parent_account_fallback" => self.parent_account_preference(true).await,
            "eligibility_never_holds_or_exhausts" => {
                self.eligibility_never_holds_or_exhausts().await
            }
            "unknown_pool_accepted" => self.unknown_pool_accepted().await,
            "malformed_pool_refused_by_field" => self.malformed_pool_refused_by_field().await,
            "pool_body_admission_conflict" => self.member_admission_conflict("pool").await,
            "parent_body_admission_conflict" => {
                self.member_admission_conflict("parent_conversation_key")
                    .await
            }
            "new_report_members_conflict" => self.new_report_members_conflict().await,
            "harness_only_when_declared" => self.harness_only_when_declared().await,
            "requested_harness_is_hosted_and_echoed" => {
                self.requested_harness_is_hosted_and_echoed().await
            }
            "unrequested_harness_uses_declaration" => {
                self.unrequested_harness_uses_declaration().await
            }
            "non_starting_runner_never_selected" => self.non_starting_runner_never_selected().await,
            "undeclared_harness_group_refuses_member" => {
                self.undeclared_harness_group_refuses_member().await
            }
            other => Err(format!("the kit has no check named {other}")),
        }
    }

    async fn fresh(&self) -> Check {
        let subject = self.subject;
        subject.reset().await.map_err(|e| format!("reset: {e}"))?;
        subject
            .configure_consumers(&fixture::consumers())
            .await
            .map_err(|e| format!("configure_consumers: {e}"))?;
        subject
            .configure_catalog(&fixture::catalog())
            .await
            .map_err(|e| format!("configure_catalog: {e}"))
    }

    async fn route(&self, stamp: RouteStamp) -> Result<S::Route, String> {
        self.subject
            .route(&stamp)
            .await
            .map_err(|e| format!("opening a route as {stamp:?}: {e}"))
    }

    /// An unscoped route as `principal`.
    async fn route_as(&self, principal: &str) -> Result<S::Route, String> {
        self.route(RouteStamp::unscoped(principal)).await
    }

    async fn describe_lists_consumers_and_fields(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "role.describe").await?;
        check_describe(&describe).map_err(|problem| format!("role.describe: {problem}"))?;
        let configured = fixture::consumers();
        for consumer in &configured {
            match describe.consumer(&consumer.id) {
                None => {
                    return Err(format!(
                        "role.describe does not list the configured consumer {}",
                        consumer.id
                    ))
                }
                Some(entry) if entry.priced_only != consumer.priced_only => {
                    return Err(format!(
                        "role.describe states priced_only {} for consumer {}, configured {}",
                        entry.priced_only, consumer.id, consumer.priced_only
                    ))
                }
                Some(_) => {}
            }
        }
        if let Some(extra) = describe
            .consumers
            .iter()
            .find(|entry| !configured.iter().any(|consumer| consumer.id == entry.id))
        {
            return Err(format!(
                "role.describe lists consumer {}, which is not configured",
                extra.id
            ));
        }
        Ok(())
    }

    async fn unknown_requirement_refused_by_name(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let params = json!({"consumer": SUMMARY, "requirements": {"iq": IQ, "gpu_count": 2}});
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &params,
            RefusalCode::InvalidRequirements,
            "a select with the unknown requirement gpu_count",
        )
        .await?;
        match refusal.detail.get("field") {
            Some(field) if field == "gpu_count" => Ok(()),
            other => Err(format!(
                "invalid_requirements for gpu_count names detail.field {other:?}, not \"gpu_count\""
            )),
        }
    }

    async fn unknown_consumer_refused(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let params = with(base(), "consumer", json!(fixture::UNCONFIGURED));
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &params,
            RefusalCode::UnknownConsumer,
            "a select for an unconfigured consumer",
        )
        .await?;
        match refusal.detail.get("consumer") {
            Some(consumer) if consumer == fixture::UNCONFIGURED => Ok(()),
            other => Err(format!(
                "unknown_consumer names detail.consumer {other:?}, not {:?}",
                fixture::UNCONFIGURED
            )),
        }
    }

    async fn declarations(&self) -> Check {
        self.declaration_entries(&fixture::catalog_declarations())
            .await
    }

    async fn declaration_entries(&self, modules: &[CatalogModule]) -> Check {
        self.subject
            .configure_catalog_declarations(modules)
            .await
            .map_err(|e| format!("configure_catalog_declarations: {e}"))
    }

    async fn declared_consumer_owner_uses_default(&self) -> Check {
        self.declarations().await?;
        let route = self.route_as(fixture::DECLARING_PRINCIPAL).await?;
        let params = json!({"consumer": fixture::DECLARED, "requirements": {"iq": IQ}});
        let selected = select(&route, &params, "the declaring module's select").await?;
        let model = fixture::catalog()
            .models
            .into_iter()
            .find(|model| model.route_key() == selected.route.route_key())
            .ok_or("the answer names a model outside the fixture catalog")?;
        if model.max_output_tokens < 50_000 || selected.max_output_tokens != model.max_output_tokens
        {
            return Err("the declaring module's select ignored the declared max_output_tokens default 50000".into());
        }
        // A request value must win even when the declaration's floor would
        // make the explicitly pinned model unavailable.
        let params = json!({"consumer": fixture::DECLARED,
            "requirements": {"iq": IQ, "max_output_tokens": 16_000},
            "pin": format!("{}/model-a", fixture::PRICED_PROVIDER)});
        let selected = select(&route, &params, "a request overriding the declared default").await?;
        if selected.route.pin() != params["pin"].as_str().unwrap() {
            return Err(
                "the request overriding the declaration did not receive its pinned model".into(),
            );
        }
        Ok(())
    }

    async fn declared_consumer_scope_owner_admitted(&self) -> Check {
        self.declarations().await?;
        let route = self
            .route(RouteStamp::scoped(
                fixture::DELEGATE_PRINCIPAL,
                ScopeStamp {
                    owner: fixture::DECLARING_PRINCIPAL.into(),
                    scope_ref: "example-run".into(),
                    scope_epoch: 7,
                },
            ))
            .await?;
        let params = json!({"consumer": fixture::DECLARED,
            "requirements": {"iq": IQ, "max_output_tokens": 16_000}});
        select(
            &route,
            &params,
            "a third-party select carrying the declaring module's scope",
        )
        .await?;
        Ok(())
    }

    async fn declared_consumer_non_owner_refused(&self) -> Check {
        self.declarations().await?;
        let own_scope = RouteStamp::scoped(
            fixture::DELEGATE_PRINCIPAL,
            ScopeStamp {
                owner: fixture::DELEGATE_PRINCIPAL.into(),
                scope_ref: "example-run".into(),
                scope_epoch: 7,
            },
        );
        for stamp in [RouteStamp::unscoped(fixture::DELEGATE_PRINCIPAL), own_scope] {
            let route = self.route(stamp).await?;
            let params = json!({"consumer": fixture::DECLARED,
                "requirements": {"iq": IQ, "max_output_tokens": 16_000}});
            let refusal = refused(
                &route,
                ops::ROUTE_SELECT,
                &params,
                RefusalCode::ConsumerNotOwned,
                "a non-owner's select without the declaring module's scope",
            )
            .await?;
            if refusal.detail.get("consumer") != Some(&json!(fixture::DECLARED))
                || refusal.wait_until_ms().is_some()
                || refusal.detail.contains_key("reset_at_ms")
            {
                return Err(
                    "consumer_not_owned must name the consumer and be terminal, never timed".into(),
                );
            }
        }
        Ok(())
    }

    async fn declared_consumer_unverified_entry_ignored(&self) -> Check {
        let mut unverified = fixture::catalog_declarations();
        unverified[0].nonce_verified = false;
        self.declaration_entries(&unverified).await?;
        let params = json!({"consumer": fixture::DECLARED,
            "requirements": {"iq": IQ}});
        let route = self.route_as(fixture::DECLARING_PRINCIPAL).await?;
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &params,
            RefusalCode::UnknownConsumer,
            "a select relying only on an unverified declaration",
        )
        .await?;
        if refusal.detail.get("consumer") != Some(&json!(fixture::DECLARED)) {
            return Err("unknown_consumer must name the unverified declaration's consumer".into());
        }
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "role.describe").await?;
        if describe.consumer(fixture::DECLARED).is_some() {
            return Err(
                "role.describe lists a consumer known only from an unverified declaration".into(),
            );
        }
        let params = with(
            params,
            "pin",
            json!(format!("{}/model-a", fixture::PRICED_PROVIDER)),
        );

        self.fresh().await?;
        let mut consumers = fixture::consumers();
        consumers.push(ConsumerConfig {
            id: fixture::DECLARED.into(),
            priced_only: false,
            requirements: Default::default(),
        });
        self.subject
            .configure_consumers(&consumers)
            .await
            .map_err(|e| format!("configure_consumers: {e}"))?;
        self.declaration_entries(&unverified).await?;
        // The unverified output floor excludes model-a. With no request or
        // operator floor, this pin succeeds only if that default is ignored.
        for principal in [fixture::DECLARING_PRINCIPAL, fixture::DELEGATE_PRINCIPAL] {
            let route = self.route_as(principal).await?;
            let selected = select(
                &route,
                &params,
                "a configured consumer alongside an unverified declaration",
            )
            .await?;
            if selected.route.pin() != params["pin"].as_str().unwrap()
                || selected.max_output_tokens != 32_000
            {
                return Err("an unverified declaration changed the configured consumer's pinned model or limits".into());
            }
        }

        let mut verified = fixture::catalog_declarations().remove(0);
        verified.principal = fixture::DELEGATE_PRINCIPAL.into();
        verified.role_declarations.insert(
            cortexkit_role_routing::ROLE_DECLARATIONS_KEY.into(),
            json!({"consumers": [{"name": fixture::DECLARED, "max_output_tokens": 16_000}]}),
        );
        // Catalog ordering cannot let an unverified claimant steal a verified
        // module's name or replace that module's declared output floor.
        for modules in [
            vec![unverified[0].clone(), verified.clone()],
            vec![verified.clone(), unverified[0].clone()],
        ] {
            self.fresh().await?;
            self.declaration_entries(&modules).await?;
            let owner = self.route_as(fixture::DELEGATE_PRINCIPAL).await?;
            let selected = select(
                &owner,
                &params,
                "the verified owner alongside an unverified claimant",
            )
            .await?;
            if selected.route.pin() != params["pin"].as_str().unwrap() {
                return Err("the verified owner did not receive its pinned model".into());
            }
        }
        Ok(())
    }

    async fn declared_consumer_conflict_refused(&self) -> Check {
        let mut modules = fixture::catalog_declarations();
        let mut other = modules[0].clone();
        other.principal = fixture::DELEGATE_PRINCIPAL.into();
        modules.push(other);
        // Configuration does not bypass conflicting verified ownership.
        let mut consumers = fixture::consumers();
        consumers.push(ConsumerConfig {
            id: fixture::DECLARED.into(),
            priced_only: false,
            requirements: Default::default(),
        });
        self.subject
            .configure_consumers(&consumers)
            .await
            .map_err(|e| format!("configure_consumers: {e}"))?;
        self.declaration_entries(&modules).await?;
        let params = json!({"consumer": fixture::DECLARED,
            "requirements": {"iq": IQ, "max_output_tokens": 16_000}});
        for principal in [fixture::DECLARING_PRINCIPAL, fixture::DELEGATE_PRINCIPAL] {
            let route = self.route_as(principal).await?;
            let refusal = refused(
                &route,
                ops::ROUTE_SELECT,
                &params,
                RefusalCode::ConsumerNotOwned,
                "a verified declarer's select for a conflicted consumer",
            )
            .await?;
            if refusal.detail.get("consumer") != Some(&json!(fixture::DECLARED))
                || refusal.wait_until_ms().is_some()
                || refusal.detail.contains_key("reset_at_ms")
            {
                return Err(
                    "a conflicted consumer must be named in terminal consumer_not_owned".into(),
                );
            }
            let describe: RoleDescribe =
                answer(&route, ops::ROLE_DESCRIBE, &json!({}), "role.describe").await?;
            if describe.consumer(fixture::DECLARED).is_some() {
                return Err(
                    "role.describe lists a consumer with conflicting verified owners".into(),
                );
            }
        }
        Ok(())
    }

    async fn operator_config_overrides_declared_default(&self) -> Check {
        self.declarations().await?;
        let mut consumers = fixture::consumers();
        consumers.push(ConsumerConfig {
            id: fixture::DECLARED.into(),
            priced_only: false,
            requirements: OperatorRequirements {
                iq: Some(IQ),
                defaults: cortexkit_role_routing::DemandDefaults {
                    max_output_tokens: Some(16_000),
                    ..Default::default()
                },
            },
        });
        self.subject
            .configure_consumers(&consumers)
            .await
            .map_err(|e| format!("configure_consumers: {e}"))?;
        let route = self.route_as(fixture::DECLARING_PRINCIPAL).await?;
        let params = json!({"consumer": fixture::DECLARED,
            "requirements": {"iq": 100, "max_output_tokens": 80_000},
            "pin": format!("{}/model-a", fixture::PRICED_PROVIDER)});
        let selected = select(
            &route,
            &params,
            "a select whose operator config overrides request and declaration",
        )
        .await?;
        if selected.route.pin() != params["pin"].as_str().unwrap() {
            return Err("operator config did not make the pinned model available".into());
        }
        Ok(())
    }

    async fn exclude_honoured(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let natural = select(&route, &base(), "a plain select").await?;
        let key = natural.route.route_key();
        let params = with(base(), "exclude", json!([key]));
        let excluded = select(&route, &params, &format!("a select excluding {key}")).await?;
        if excluded.route.route_key() == key {
            return Err(format!("a select excluding {key} was answered with {key}"));
        }
        Ok(())
    }

    async fn runners_honoured(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let natural = select(&route, &base(), "a plain select").await?;
        let other = another_runner(&natural.runner.module)?;
        let params = with(base(), "runners", json!([other]));
        let answer = select(&route, &params, &format!("a select with runners [{other}]")).await?;
        if answer.runner.module != other {
            return Err(format!(
                "a select with runners [{other}] was answered on runner {}",
                answer.runner.module
            ));
        }
        Ok(())
    }

    async fn timed_refusals_carry_reset_at_ms(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let now = now_ms();
        let until = now + 600_000;
        let catalog = fixture::catalog();
        let cooled = catalog.models[0].route_key();
        self.subject
            .cool_down(&cooled, until)
            .await
            .map_err(|e| format!("cool_down: {e}"))?;

        let what = format!("a pin on the cooled-down route {cooled}");
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &with(base(), "pin", json!(cooled)),
            RefusalCode::PinUnavailable,
            &what,
        )
        .await?;
        let detail: PinUnavailableDetail = refusal.detail_as().ok_or_else(|| {
            format!(
                "{what}: the pin_unavailable detail does not decode: {:?}",
                refusal.detail
            )
        })?;
        if detail.reason != PinUnavailableReason::Cooldown {
            return Err(format!(
                "{what} is pin_unavailable reason {}, not cooldown",
                detail.reason
            ));
        }
        match detail.reset_at_ms {
            None => return Err(format!("{what} is pin_unavailable reason cooldown, a timed cause, but carries no reset_at_ms")),
            Some(reset) if reset <= now => {
                return Err(format!("{what}: reset_at_ms {reset} is not in the future (now {now})"))
            }
            Some(_) => {}
        }

        let unknown = format!("{}/model-z", fixture::PRICED_PROVIDER);
        let what = format!("a pin on the unknown model {unknown}");
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &with(base(), "pin", json!(unknown)),
            RefusalCode::PinUnavailable,
            &what,
        )
        .await?;
        let detail: PinUnavailableDetail = refusal.detail_as().ok_or_else(|| {
            format!(
                "{what}: the pin_unavailable detail does not decode: {:?}",
                refusal.detail
            )
        })?;
        if detail.reason != PinUnavailableReason::UnknownModel {
            return Err(format!(
                "{what} is pin_unavailable reason {}, not unknown_model",
                detail.reason
            ));
        }
        if let Some(reset) = detail.reset_at_ms {
            return Err(format!(
                "{what} is not a timed cause, but carries reset_at_ms {reset}"
            ));
        }

        for model in &catalog.models[1..] {
            self.subject
                .cool_down(&model.route_key(), until)
                .await
                .map_err(|e| format!("cool_down: {e}"))?;
        }
        let what = "a select while every route is cooling down";
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &base(),
            RefusalCode::NoRoute,
            what,
        )
        .await?;
        let detail: NoRouteDetail = refusal.detail_as().ok_or_else(|| {
            format!(
                "{what}: the no_route detail does not decode: {:?}",
                refusal.detail
            )
        })?;
        if detail.reason != NoRouteReason::QuotaOrCooldown {
            return Err(format!(
                "{what} is no_route reason {}, not quota_or_cooldown",
                detail.reason
            ));
        }
        match detail.reset_at_ms {
            None => Err(format!("{what} is no_route reason quota_or_cooldown, a timed cause, but carries no reset_at_ms")),
            Some(reset) if reset <= now => {
                Err(format!("{what}: reset_at_ms {reset} is not in the future (now {now})"))
            }
            Some(_) => Ok(()),
        }
    }

    async fn scope_comes_from_route_stamp(&self) -> Check {
        let unscoped = self.route_as(CONSUMER).await?;
        let natural = select(&unscoped, &base(), "a plain select").await?;
        let pinned = another_route_key(&natural.route.route_key())?;
        self.subject
            .pin_for_project(fixture::PINNED_PROJECT, SUMMARY, &pinned)
            .await
            .map_err(|e| format!("pin_for_project: {e}"))?;
        let project = fixture::PINNED_PROJECT;

        let bound = self.route(RouteStamp::project(CONSUMER, project)).await?;
        let answer = select(
            &bound,
            &base(),
            &format!("a select on a route bound to {project}"),
        )
        .await?;
        if answer.route.route_key() != pinned {
            return Err(format!(
                "under a route bound to {project}, the project's pin {pinned} was not applied: answered {}",
                answer.route.route_key()
            ));
        }

        let answer = select(&unscoped, &base(), "a select on an unscoped route").await?;
        if answer.route.route_key() == pinned {
            return Err(format!(
                "on an unscoped route, {project}'s pin {pinned} was applied"
            ));
        }

        // A body naming the pinned project must not reach its pin: scope
        // is read from the route only. The provider may ignore or refuse
        // these members; either keeps the rule.
        let body = with(
            with(base(), "project_id", json!(project)),
            "scope",
            json!({"project_id": project}),
        );
        let other = self
            .route(RouteStamp::project(CONSUMER, fixture::OTHER_PROJECT))
            .await?;
        for (route, label) in [
            (&unscoped, "an unscoped route".to_owned()),
            (
                &other,
                format!("a route bound to {}", fixture::OTHER_PROJECT),
            ),
        ] {
            if let Ok(answer) = call::<_, SelectAnswer>(route, ops::ROUTE_SELECT, &body).await? {
                if answer.route.route_key() == pinned {
                    return Err(format!(
                        "on {label}, a body naming project {project} got that project's pin {pinned}: scope was read from the body"
                    ));
                }
            }
        }
        Ok(())
    }

    async fn report_from_other_principal_unknown_decision(&self) -> Check {
        let consumer = self.route_as(CONSUMER).await?;
        let other = self.route_as(fixture::OTHER_CONSUMER).await?;
        let decision = select(&consumer, &base(), "a plain select")
            .await?
            .decision_id;
        let report = json!({"decision_id": decision, "outcome": "completed"});
        let foreign = refused(
            &other,
            ops::ROUTE_REPORT,
            &report,
            RefusalCode::UnknownDecision,
            "a report from another principal on its decision",
        )
        .await?;
        let never_made = refused(
            &other,
            ops::ROUTE_REPORT,
            &json!({"decision_id": NEVER_MADE, "outcome": "completed"}),
            RefusalCode::UnknownDecision,
            "a report on a decision that never existed",
        )
        .await?;
        same_refusal(&foreign, &never_made, "a report from another principal")?;
        let own: ReportAnswer = answer(
            &consumer,
            ops::ROUTE_REPORT,
            &report,
            "the selecting principal's report, after another principal's was refused",
        )
        .await?;
        if own.decision_id != decision {
            return Err(format!(
                "the report on {decision} answered decision_id {}",
                own.decision_id
            ));
        }
        Ok(())
    }

    async fn repeated_report_returns_first_answer(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let decision = select(&route, &base(), "a plain select").await?.decision_id;
        let report = json!({
            "decision_id": decision,
            "outcome": "completed",
            "usage": {"input_tokens": 100, "output_tokens": 20}
        });
        let first: Value = answer(&route, ops::ROUTE_REPORT, &report, "a report").await?;
        let _: ReportAnswer = serde_json::from_value(first.clone())
            .map_err(|e| format!("the report answer does not decode: {e}"))?;
        let again: Value =
            answer(&route, ops::ROUTE_REPORT, &report, "the same report again").await?;
        if again != first {
            return Err(format!(
                "the same report sent again answered {again}, but the first answer was {first}"
            ));
        }
        Ok(())
    }

    async fn different_report_is_report_conflict(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let decision = select(&route, &base(), "a plain select").await?.decision_id;
        let completed = json!({"decision_id": decision, "outcome": "completed"});
        let _: ReportAnswer = answer(&route, ops::ROUTE_REPORT, &completed, "a report").await?;
        refused(
            &route,
            ops::ROUTE_REPORT,
            &json!({"decision_id": decision, "outcome": "failed"}),
            RefusalCode::ReportConflict,
            "a report with outcome failed after one with completed",
        )
        .await?;
        Ok(())
    }

    async fn affinity_holds_until_failure_report(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let natural = select(&route, &base(), "a plain select").await?;
        let kept = another_route_key(&natural.route.route_key())?;
        let key = "affinity-check";
        let keyed = with(base(), "affinity", json!(key));

        let first = select(
            &route,
            &with(keyed.clone(), "pin", json!(kept)),
            &format!("a select pinned to {kept} with affinity {key}"),
        )
        .await?;
        if first.route.route_key() != kept {
            return Err(format!(
                "a select pinned to {kept} was answered {}",
                first.route.route_key()
            ));
        }
        let held = select(&route, &keyed, &format!("a select with affinity {key}")).await?;
        if held.route.route_key() != kept {
            return Err(format!(
                "after a decision on {kept}, a select with affinity {key} was answered {}: affinity did not keep the route",
                held.route.route_key()
            ));
        }
        let _: ReportAnswer = answer(
            &route,
            ops::ROUTE_REPORT,
            &json!({"decision_id": held.decision_id, "outcome": "failed", "runner_code": "provider_error"}),
            "a failure report",
        )
        .await?;
        let after = select(
            &route,
            &keyed,
            &format!("a select with affinity {key} after a failure report"),
        )
        .await?;
        if after.route.route_key() == kept {
            return Err(format!(
                "after a failure report on {kept}, a select with affinity {key} was still answered {kept}"
            ));
        }
        Ok(())
    }

    async fn answer_carries_limits_and_future_expiry(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let now = now_ms();
        let answer = select(&route, &base(), "a plain select").await?;
        if answer.decision_id.is_empty() {
            return Err("the answer's decision_id is empty".to_owned());
        }
        let catalog = fixture::catalog();
        let key = answer.route.route_key();
        let model = catalog
            .models
            .iter()
            .find(|model| model.route_key() == key)
            .ok_or_else(|| format!("the answered route {key} is not in the catalog"))?;
        if answer.context_window != model.context_window
            || answer.max_output_tokens != model.max_output_tokens
        {
            return Err(format!(
                "the answer for {key} carries context_window {} and max_output_tokens {}, the catalog says {} and {}",
                answer.context_window,
                answer.max_output_tokens,
                model.context_window,
                model.max_output_tokens
            ));
        }
        let runs = catalog.runners.iter().any(|runner| {
            runner.module == answer.runner.module && runner.route_keys.contains(&key)
        });
        if !runs {
            return Err(format!(
                "the answer names runner {}, which does not run {key}",
                answer.runner.module
            ));
        }
        if answer.expires_at_ms <= now {
            return Err(format!(
                "expires_at_ms {} is not in the future (now {now})",
                answer.expires_at_ms
            ));
        }
        if answer.explanation.trim().is_empty() {
            return Err("the answer's explanation is empty".to_owned());
        }
        Ok(())
    }

    fn account_for(admission: &str, conversation: &str, model: &str) -> Value {
        json!({
            "provider": fixture::SUBSCRIPTION_PROVIDER,
            "model": model,
            "conversation_key": conversation,
            "admission_id": admission,
        })
    }

    async fn parent_account_preference(&self, fallback: bool) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let parent: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for("parent-admission", "parent", "model-c"),
            "parent admission",
        )
        .await?;
        let mut child = Self::account_for("child-admission", "child", "model-c");
        child["parent_conversation_key"] = json!("parent");
        if fallback {
            child["candidates"] = json!({"view_digest": "only-parent", "accounts": [{
                "account_id": parent.account.account_id, "credential_id": parent.account.credential_id
            }]});
        }
        let chosen: AccountForAnswer =
            answer(&runner, ops::ROUTE_ACCOUNT_FOR, &child, "child admission").await?;
        if (chosen.account.account_id == parent.account.account_id) != fallback {
            return Err(if fallback {
                "did not fall back to the parent's only eligible account"
            } else {
                "picked the parent's account despite another eligible account"
            }
            .into());
        }
        Ok(())
    }

    async fn eligibility_never_holds_or_exhausts(&self) -> Check {
        let consumer = self.route_as(CONSUMER).await?;
        let selected = select(
            &consumer,
            &with(
                base(),
                "pin",
                json!(format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER)),
            ),
            "subscription select",
        )
        .await?;
        let runner = self.route_as(&selected.runner.module).await?;
        let admitted: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for("eligibility-admission", "eligibility", "model-c"),
            "initial admission",
        )
        .await?;
        let _: ReportAnswer = answer(
            &consumer,
            ops::ROUTE_REPORT,
            &json!({
                "decision_id": selected.decision_id, "outcome": "refused",
                "credential_id": admitted.account.credential_id,
                "eligibility": {"scope": "model", "code": "model_not_included"}
            }),
            "model eligibility report",
        )
        .await?;
        // A different model on the same account remains eligible. An account-level
        // quota refusal or hold would incorrectly refuse this call.
        let mut next = Self::account_for("after-eligibility", "other-model", "model-d");
        next["candidates"] = json!({"view_digest": "same-account", "accounts": [{
            "account_id": admitted.account.account_id, "credential_id": admitted.account.credential_id
        }]});
        let after: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &next,
            "another model after eligibility report",
        )
        .await?;
        if after.account.account_id != admitted.account.account_id {
            return Err("another model did not retain access to the eligible account".into());
        }
        Ok(())
    }

    async fn unknown_pool_accepted(&self) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let body = with(
            Self::account_for("new-pool", "new-pool", "model-c"),
            "pool",
            json!("never-seen.pool_7-x"),
        );
        let _: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &body,
            "an unseen well-formed pool",
        )
        .await?;
        let _: AccountObserveAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_OBSERVE,
            &json!({"event_id": "new-pool", "kind": "attempt", "pool": "another-unseen.pool"}),
            "an unseen observation pool",
        )
        .await?;
        let consumer = self.route_as(CONSUMER).await?;
        let selected = select(&consumer, &base(), "select for pool report").await?;
        let _: ReportAnswer = answer(&consumer, ops::ROUTE_REPORT,
            &json!({"decision_id": selected.decision_id, "outcome": "completed", "pool": "unseen-report.pool"}), "an unseen report pool").await?;
        Ok(())
    }

    async fn malformed_pool_refused_by_field(&self) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let consumer = self.route_as(CONSUMER).await?;
        let selected = select(&consumer, &base(), "select for malformed pool report").await?;
        for pool in [
            json!(""),
            json!("Upper"),
            json!("a/b"),
            json!("a".repeat(65)),
            json!(7),
        ] {
            for (route, op, mut body) in [
                (
                    &runner,
                    ops::ROUTE_ACCOUNT_FOR,
                    Self::account_for("bad-pool", "bad-pool", "model-c"),
                ),
                (
                    &runner,
                    ops::ROUTE_ACCOUNT_OBSERVE,
                    json!({"event_id": "bad-pool", "kind": "attempt"}),
                ),
                (
                    &consumer,
                    ops::ROUTE_REPORT,
                    json!({"decision_id": selected.decision_id, "outcome": "completed"}),
                ),
            ] {
                body["pool"] = pool.clone();
                let refusal = refused(
                    route,
                    op,
                    &body,
                    RefusalCode::InvalidRequest,
                    "a malformed pool",
                )
                .await?;
                if refusal.detail.get("field") != Some(&json!("pool")) {
                    return Err(format!(
                        "{op}: malformed pool must name detail.field pool, got {:?}",
                        refusal.detail
                    ));
                }
            }
        }
        Ok(())
    }

    async fn member_admission_conflict(&self, member: &str) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let body = Self::account_for("member-conflict", "member-conflict", "model-c");
        let _: AccountForAnswer =
            answer(&runner, ops::ROUTE_ACCOUNT_FOR, &body, "initial admission").await?;
        for value in ["one", "two"] {
            refused(
                &runner,
                ops::ROUTE_ACCOUNT_FOR,
                &with(body.clone(), member, json!(value)),
                RefusalCode::AdmissionConflict,
                &format!("changed {member} under reused admission_id"),
            )
            .await?;
        }
        Ok(())
    }

    async fn new_report_members_conflict(&self) -> Check {
        let consumer = self.route_as(CONSUMER).await?;
        for (member, first_value, second_value) in [
            ("pool", json!("one"), json!("two")),
            (
                "eligibility",
                json!({"scope": "model", "code": "one"}),
                json!({"scope": "model", "code": "two"}),
            ),
            (
                "eligibility",
                json!({"scope": "model", "code": "one"}),
                json!({"scope": "account", "code": "one"}),
            ),
            ("request_input_tokens", json!(0), json!(1)),
        ] {
            let selected = select(&consumer, &base(), "select for report body comparison").await?;
            let body = with(
                json!({"decision_id": selected.decision_id, "outcome": "refused"}),
                member,
                first_value,
            );
            let _: ReportAnswer =
                answer(&consumer, ops::ROUTE_REPORT, &body, "initial report").await?;
            refused(
                &consumer,
                ops::ROUTE_REPORT,
                &with(body, member, second_value),
                RefusalCode::ReportConflict,
                &format!("changed {member} under reused decision_id"),
            )
            .await?;
        }
        Ok(())
    }

    async fn account_for_idempotent_on_admission_id(&self) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let body = Self::account_for("admission-replay", "conversation-replay", "model-c");
        let first: Value = answer(&runner, ops::ROUTE_ACCOUNT_FOR, &body, "an admission").await?;
        let decoded: AccountForAnswer = serde_json::from_value(first.clone())
            .map_err(|e| format!("the account_for answer does not decode: {e}"))?;
        if decoded.account.provider != fixture::SUBSCRIPTION_PROVIDER {
            return Err(format!(
                "an admission for {} was answered with an account of {}",
                fixture::SUBSCRIPTION_PROVIDER,
                decoded.account.provider
            ));
        }
        let again: Value = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &body,
            "the same admission again",
        )
        .await?;
        if again != first {
            return Err(format!(
                "the same admission sent again answered {again}, but the first answer was {first}"
            ));
        }
        Ok(())
    }

    async fn account_for_different_body_admission_conflict(&self) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let _: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for("admission-conflict", "conversation-1", "model-c"),
            "an admission",
        )
        .await?;
        refused(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for("admission-conflict", "conversation-2", "model-c"),
            RefusalCode::AdmissionConflict,
            "the same admission_id with another conversation_key",
        )
        .await?;
        Ok(())
    }

    async fn account_for_foreign_decision_unknown(&self) -> Check {
        let consumer = self.route_as(CONSUMER).await?;
        let pin = format!("{}/model-c", fixture::SUBSCRIPTION_PROVIDER);
        let decision = select(
            &consumer,
            &with(base(), "pin", json!(pin)),
            &format!("a select pinned to {pin}"),
        )
        .await?;
        let named = decision.runner.module.clone();
        let other = another_runner(&named)?;
        let named_route = self.route_as(&named).await?;
        let other_route = self.route_as(&other).await?;
        let body = |admission: &str, model: &str, decision_id: &str| {
            with(
                Self::account_for(admission, "conversation-decision", model),
                "decision_id",
                json!(decision_id),
            )
        };
        let id = decision.decision_id.as_str();

        let never_made = refused(
            &named_route,
            ops::ROUTE_ACCOUNT_FOR,
            &body("admission-never-made", "model-c", NEVER_MADE),
            RefusalCode::UnknownDecision,
            "an admission naming a decision that never existed",
        )
        .await?;
        let what = format!("an admission from {other} naming a decision for runner {named}");
        let foreign_runner = refused(
            &other_route,
            ops::ROUTE_ACCOUNT_FOR,
            &body("admission-other-runner", "model-c", id),
            RefusalCode::UnknownDecision,
            &what,
        )
        .await?;
        same_refusal(&foreign_runner, &never_made, &what)?;
        let what = format!("an admission for model-d naming a decision for {pin}");
        let foreign_model = refused(
            &named_route,
            ops::ROUTE_ACCOUNT_FOR,
            &body("admission-other-model", "model-d", id),
            RefusalCode::UnknownDecision,
            &what,
        )
        .await?;
        same_refusal(&foreign_model, &never_made, &what)?;

        let own: AccountForAnswer = answer(
            &named_route,
            ops::ROUTE_ACCOUNT_FOR,
            &body("admission-own", "model-c", id),
            &format!("an admission from {named} for model-c naming its decision"),
        )
        .await?;
        if own.reason.decision_id != id {
            return Err(format!(
                "an admission naming decision {id} answered reason.decision_id {}",
                own.reason.decision_id
            ));
        }
        Ok(())
    }

    async fn account_observe_idempotent_per_event_id(&self) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let body =
            json!({"event_id": "runner-one-log:1", "kind": "attempt", "observed_at_ms": now_ms()});
        let first: AccountObserveAnswer =
            answer(&runner, ops::ROUTE_ACCOUNT_OBSERVE, &body, "an observation").await?;
        if first.observation.duplicate {
            return Err("the first observation of an event_id answered duplicate true".to_owned());
        }
        let again: AccountObserveAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_OBSERVE,
            &body,
            "the same observation again",
        )
        .await?;
        if !again.observation.duplicate {
            return Err("the same event_id sent again answered duplicate false".to_owned());
        }
        if again.observation.id != first.observation.id {
            return Err(format!(
                "the same event_id sent again answered observation {}, the first was {}",
                again.observation.id, first.observation.id
            ));
        }
        Ok(())
    }

    async fn tool_capabilities_matched_by_tag(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let needing = |tag: &str| json!({"consumer": SUMMARY, "requirements": {"iq": IQ, "tool_capabilities": [tag]}});
        for (tag, runner, tools) in [
            (fixture::TAG_ONE, fixture::RUNNER_ONE, fixture::TOOLS_ONE),
            (fixture::TAG_TWO, fixture::RUNNER_TWO, fixture::TOOLS_TWO),
        ] {
            let what = format!("a select requiring the tool capability {tag}");
            let answer =
                match call::<_, SelectAnswer>(&route, ops::ROUTE_SELECT, &needing(tag)).await? {
                    Ok(answer) => answer,
                    Err(refusal) => {
                        return Err(format!(
                            "{what} was refused {}; {runner} reaches {tools}, which serves it",
                            refusal.code
                        ))
                    }
                };
            if answer.runner.module != runner {
                return Err(format!(
                    "{what} was answered on {}; only {runner} reaches a tool provider serving it",
                    answer.runner.module
                ));
            }
        }
        let module = fixture::TOOLS_ONE;
        refused(
            &route,
            ops::ROUTE_SELECT,
            &needing(module),
            RefusalCode::NoRoute,
            &format!("a select listing the tool provider module name {module} as a tool capability (tags are matched, never module names)"),
        )
        .await?;
        Ok(())
    }

    async fn harness_only_when_declared(&self) -> Check {
        // Each runner runs one priced model, so a pin on each is answered on
        // a different runner.
        let mut catalog = fixture::catalog();
        let keys: Vec<String> = catalog.models[..2]
            .iter()
            .map(|model| model.route_key())
            .collect();
        for (runner, key) in catalog.runners.iter_mut().zip(&keys) {
            runner.route_keys = vec![key.clone()];
        }
        self.subject
            .configure_catalog(&catalog)
            .await
            .map_err(|e| format!("configure_catalog: {e}"))?;
        let route = self.route_as(CONSUMER).await?;
        for pin in &keys {
            let what = format!("a select pinned to {pin}");
            let answer = select(&route, &with(base(), "pin", json!(pin)), &what).await?;
            let runner = &answer.runner.module;
            let declared = catalog
                .runners
                .iter()
                .find(|candidate| &candidate.module == runner)
                .ok_or_else(|| {
                    format!("{what} was answered on {runner}, which the catalog does not list")
                })?
                .default_harness
                .as_deref();
            let harness = answer.runner.harness.as_deref();
            if harness != declared {
                return Err(match declared {
                    Some(declared) => format!(
                        "{what} was answered with harness {harness:?}; {runner} declared {declared:?}"
                    ),
                    None => format!(
                        "{what} was answered with harness {harness:?}; {runner} declared none"
                    ),
                });
            }
        }
        Ok(())
    }

    async fn requested_harness_is_hosted_and_echoed(&self) -> Check {
        let mut catalog = fixture::harness_catalog();
        catalog.runners.remove(0);
        self.subject.configure_catalog(&catalog).await?;
        let route = self.route_as(CONSUMER).await?;
        for (harness, runner) in [("beta", fixture::RUNNER_ONE), ("solo", fixture::RUNNER_TWO)] {
            let answer = select(
                &route,
                &with(base(), "harness", json!(harness)),
                "requested harness",
            )
            .await?;
            if answer.runner.module != runner || answer.runner.harness.as_deref() != Some(harness) {
                return Err(format!(
                    "requested {harness} must be hosted and echoed by {runner}, got {:?}",
                    answer.runner
                ));
            }
        }
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &with(base(), "harness", json!("unhosted")),
            RefusalCode::NoRoute,
            "unhosted harness",
        )
        .await?;
        let detail: NoRouteDetail = refusal.detail_as().ok_or("missing no_route detail")?;
        if detail.reason != NoRouteReason::HarnessUnavailable
            || !refusal.message.contains("unhosted")
        {
            return Err(format!(
                "unhosted harness needs reason harness_unavailable and its name: {refusal:?}"
            ));
        }
        Ok(())
    }

    async fn unrequested_harness_uses_declaration(&self) -> Check {
        let mut catalog = fixture::harness_catalog();
        catalog.runners.remove(0);
        let route = self.route_as(CONSUMER).await?;
        for (harnesses, default, expected) in [
            (Some(vec!["alpha", "beta"]), Some("alpha"), Some("alpha")),
            (Some(vec!["alpha", "beta"]), Some("beta"), Some("beta")),
            (Some(vec!["solo"]), None, Some("solo")),
            (Some(vec!["alpha", "beta"]), None, None),
            (None, None, None),
        ] {
            catalog.runners.truncate(1);
            catalog.runners[0].harnesses =
                harnesses.map(|values| values.into_iter().map(str::to_owned).collect());
            catalog.runners[0].default_harness = default.map(str::to_owned);
            self.subject.configure_catalog(&catalog).await?;
            let answer = select(&route, &base(), "no requested harness").await?;
            if answer.runner.module != fixture::RUNNER_ONE
                || answer.runner.harness.as_deref() != expected
            {
                return Err(format!(
                    "no requested harness: expected {expected:?} on runner-one, got {:?}",
                    answer.runner
                ));
            }
        }
        Ok(())
    }

    async fn non_starting_runner_never_selected(&self) -> Check {
        let mut catalog = fixture::harness_catalog();
        self.subject.configure_catalog(&catalog).await?;
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "role.describe").await?;
        let mut requests = vec![base()];
        if describe.serves(SELECT_HARNESS) {
            requests.push(with(base(), "harness", json!("alpha")));
        }
        for request in &requests {
            let answer = select(&route, request, "gateway excluded").await?;
            if answer.runner.module == "runner-gateway" {
                return Err("starts_sessions false runner was selected".into());
            }
            let pin = catalog.models[0].route_key();
            let pinned = select(
                &route,
                &with(request.clone(), "pin", json!(pin)),
                "pinned gateway excluded",
            )
            .await?;
            if pinned.runner.module == "runner-gateway" {
                return Err("starts_sessions false runner was selected for a pin".into());
            }
            refused(
                &route,
                ops::ROUTE_SELECT,
                &with(request.clone(), "runners", json!(["runner-gateway"])),
                RefusalCode::NoRoute,
                "only gateway listed",
            )
            .await?;
        }
        catalog.runners.truncate(1);
        self.subject.configure_catalog(&catalog).await?;
        for request in &requests {
            refused(
                &route,
                ops::ROUTE_SELECT,
                request,
                RefusalCode::NoRoute,
                "only gateway available",
            )
            .await?;
        }
        let gateway = self.route_as("runner-gateway").await?;
        let _: AccountForAnswer = answer(
            &gateway,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for("gateway-admission", "gateway-own-turn", "model-c"),
            "gateway account choice for its own turn",
        )
        .await?;
        let _: AccountObserveAnswer = answer(
            &gateway,
            ops::ROUTE_ACCOUNT_OBSERVE,
            &json!({"event_id": "gateway-observation", "kind": "attempt"}),
            "gateway account observation",
        )
        .await?;
        Ok(())
    }

    async fn undeclared_harness_group_refuses_member(&self) -> Check {
        let route = self.route_as(CONSUMER).await?;
        let refusal = refused(
            &route,
            ops::ROUTE_SELECT,
            &with(base(), "harness", json!("alpha")),
            RefusalCode::InvalidRequest,
            "undeclared harness group",
        )
        .await?;
        if refusal.detail.get("field").and_then(Value::as_str) != Some("harness") {
            return Err(format!(
                "undeclared harness group refusal must name harness: {refusal:?}"
            ));
        }
        Ok(())
    }

    /// Run the hold check `name`, or report it not applicable when the
    /// provider does not declare `select_hold_control`.
    async fn hold_check(&self, name: &str) -> Result<Ending, String> {
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "role.describe").await?;
        if !describe.serves(SELECT_HOLD_CONTROL) {
            return Ok(Ending::NotApplicable(format!(
                "role.describe does not declare {SELECT_HOLD_CONTROL}"
            )));
        }
        self.subject
            .configure_catalog(&fixture::hold_catalog())
            .await
            .map_err(|e| format!("configure_catalog: {e}"))?;
        self.hold_limits(MAX_HOLD_S, HOLD_WHEN_RESET_WITHIN_S)
            .await?;
        let checked = match name {
            "hold_answers_blocked_entry_within_limits" => {
                self.hold_answers_blocked_entry_within_limits().await
            }
            "hold_moves_when_reset_beyond_window" => {
                self.hold_moves_when_reset_beyond_window().await
            }
            "hold_moves_when_reset_unknown" => self.hold_moves_when_reset_unknown().await,
            "hold_moves_beyond_caller_cap" => self.hold_moves_beyond_caller_cap().await,
            "hold_ok_false_moves_to_usable_entry" => {
                self.hold_ok_false_moves_to_usable_entry().await
            }
            "hold_ok_false_falls_back_to_soonest_entry" => {
                self.hold_ok_false_falls_back_to_soonest_entry().await
            }
            "hold_nothing_within_limit_accounts_blocked" => {
                self.hold_nothing_within_limit_accounts_blocked().await
            }
            "hold_demand_mode_skips_blocked_model" => {
                self.hold_demand_mode_skips_blocked_model().await
            }
            "hold_account_for_applies_recorded_cap" => {
                self.hold_account_for_applies_recorded_cap().await
            }
            other => Err(format!("the kit has no hold check named {other}")),
        };
        checked.map(|()| Ending::Passed)
    }

    /// Set the operator's hold settings for both subscription providers of
    /// the hold catalog.
    async fn hold_limits(&self, max_hold_s: u64, within_s: u64) -> Check {
        for provider in [
            fixture::SUBSCRIPTION_PROVIDER,
            fixture::SECOND_SUBSCRIPTION_PROVIDER,
        ] {
            self.subject
                .configure_hold_limits(provider, HoldLimits::new(max_hold_s, within_s))
                .await
                .map_err(|e| format!("configure_hold_limits: {e}"))?;
        }
        Ok(())
    }

    /// Route the summary consumer by a ladder of `entries`.
    async fn ladder(&self, entries: &[&str]) -> Check {
        let mode = ConsumerMode::Ladder {
            entries: entries.iter().map(|entry| (*entry).to_owned()).collect(),
        };
        self.subject
            .configure_consumer_mode(SUMMARY, &mode)
            .await
            .map_err(|e| format!("configure_consumer_mode: {e}"))
    }

    /// Block every hold-catalog account of `provider` until `reset_at_ms`,
    /// or until an unknown time.
    async fn block_provider(&self, provider: &str, reset_at_ms: Option<u64>) -> Check {
        for account in fixture::hold_catalog()
            .accounts
            .iter()
            .filter(|account| account.provider == provider)
        {
            self.subject
                .block_account(&account.account_id, reset_at_ms)
                .await
                .map_err(|e| format!("block_account: {e}"))?;
        }
        Ok(())
    }

    /// Select as the summary consumer with `members` added, and require
    /// the answer to be the route `expected`.
    async fn hold_select(
        &self,
        members: Value,
        expected: &str,
        what: &str,
    ) -> Result<SelectAnswer, String> {
        let route = self.route_as(CONSUMER).await?;
        let mut params = base();
        params
            .as_object_mut()
            .expect("base is an object")
            .extend(members.as_object().cloned().unwrap_or_default());
        let answer = select(&route, &params, what).await?;
        let key = answer.route.route_key();
        if key != expected {
            return Err(format!("{what} was answered {key}, not {expected}"));
        }
        Ok(answer)
    }

    /// `route.account_for` from the runner `answer` names, for its model,
    /// naming its decision when `decision` is set.
    async fn hold_account_for(
        &self,
        answer: &SelectAnswer,
        admission: &str,
        decision: bool,
    ) -> Result<Result<AccountForAnswer, Refusal>, String> {
        let runner = self.route_as(&answer.runner.module).await?;
        let mut params = json!({
            "provider": answer.route.provider,
            "model": answer.route.model,
            "conversation_key": format!("{admission}-conversation"),
            "admission_id": admission,
        });
        if decision {
            params["decision_id"] = json!(answer.decision_id);
        }
        call(&runner, ops::ROUTE_ACCOUNT_FOR, &params).await
    }

    async fn hold_answers_blocked_entry_within_limits(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_PRICED]).await?;
        let reset = now_ms() + 60_000;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(reset))
            .await?;
        let what = "a select whose first ladder entry is blocked for 60 s, within both limits";
        let answer = self.hold_select(json!({}), HOLD_FIRST, what).await?;
        match self.hold_account_for(&answer, "hold-admission", true).await? {
            Err(refusal) if refusal.code == RefusalCode::AccountHold => Ok(()),
            Err(refusal) => Err(format!(
                "account_for on the held entry was refused {}, not account_hold: {}",
                refusal.code, refusal.message
            )),
            Ok(admitted) => Err(format!(
                "account_for on the held entry was answered account {}, though every account is blocked",
                admitted.account.account_id
            )),
        }
    }

    async fn hold_moves_when_reset_beyond_window(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_PRICED]).await?;
        self.hold_limits(1_800, 600).await?;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now_ms() + 900_000))
            .await?;
        let what = "a select whose first ladder entry resets in 900 s, beyond hold_when_reset_within 600 s and within max_hold 1800 s,";
        self.hold_select(json!({}), HOLD_PRICED, what).await?;
        Ok(())
    }

    async fn hold_moves_when_reset_unknown(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_PRICED]).await?;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, None)
            .await?;
        let what = "a select whose first ladder entry is blocked with an unknown reset";
        self.hold_select(json!({}), HOLD_PRICED, what).await?;
        Ok(())
    }

    async fn hold_moves_beyond_caller_cap(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_PRICED]).await?;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now_ms() + 300_000))
            .await?;
        let what = "a select with hold_wait_max_s 120 whose first ladder entry resets in 300 s, within max_hold 900 s,";
        self.hold_select(json!({"hold_wait_max_s": 120}), HOLD_PRICED, what)
            .await?;
        Ok(())
    }

    async fn hold_ok_false_moves_to_usable_entry(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_PRICED]).await?;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now_ms() + 60_000))
            .await?;
        let what = "a select with hold_ok false whose first ladder entry is blocked for 60 s and whose second is usable now";
        self.hold_select(json!({"hold_ok": false}), HOLD_PRICED, what)
            .await?;
        Ok(())
    }

    async fn hold_ok_false_falls_back_to_soonest_entry(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_SECOND]).await?;
        let now = now_ms();
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now + 300_000))
            .await?;
        self.block_provider(fixture::SECOND_SUBSCRIPTION_PROVIDER, Some(now + 60_000))
            .await?;
        let what = "a select with hold_ok false whose ladder entries are both blocked, the first for 300 s and the second for 60 s,";
        self.hold_select(json!({"hold_ok": false}), HOLD_SECOND, what)
            .await?;
        Ok(())
    }

    async fn hold_nothing_within_limit_accounts_blocked(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_SECOND]).await?;
        let route = self.route_as(CONSUMER).await?;
        let params = with(base(), "hold_ok", json!(false));
        let now = now_ms();
        let earliest = now + 1_500_000;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now + 2_000_000))
            .await?;
        self.block_provider(fixture::SECOND_SUBSCRIPTION_PROVIDER, Some(earliest))
            .await?;
        for known in [true, false] {
            if !known {
                for provider in [
                    fixture::SUBSCRIPTION_PROVIDER,
                    fixture::SECOND_SUBSCRIPTION_PROVIDER,
                ] {
                    self.block_provider(provider, None).await?;
                }
            }
            let what = if known {
                "a select with hold_ok false whose ladder entries reset in 2000 s and 1500 s, beyond the 900 s limit,"
            } else {
                "a select with hold_ok false whose ladder entries are blocked with unknown resets"
            };
            let refusal = refused(
                &route,
                ops::ROUTE_SELECT,
                &params,
                RefusalCode::NoRoute,
                what,
            )
            .await?;
            let detail: NoRouteDetail = refusal.detail_as().ok_or_else(|| {
                format!(
                    "{what}: the no_route detail does not decode: {:?}",
                    refusal.detail
                )
            })?;
            if detail.reason != NoRouteReason::AccountsBlocked {
                return Err(format!(
                    "{what} is no_route reason {}, not accounts_blocked",
                    detail.reason
                ));
            }
            match (known, detail.reset_at_ms) {
                (true, Some(reset)) if reset.abs_diff(earliest) <= 1_000 => {}
                (true, other) => {
                    return Err(format!(
                    "{what} carries reset_at_ms {other:?}, not the earliest known reset {earliest}"
                ))
                }
                (false, None) => {}
                (false, Some(reset)) => {
                    return Err(format!(
                        "{what} carries reset_at_ms {reset}, though no reset is known"
                    ))
                }
            }
        }
        Ok(())
    }

    async fn hold_demand_mode_skips_blocked_model(&self) -> Check {
        self.subject
            .configure_consumer_mode(SUMMARY, &ConsumerMode::Demand)
            .await
            .map_err(|e| format!("configure_consumer_mode: {e}"))?;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now_ms() + 60_000))
            .await?;
        let what = "a demand-mode select whose first adequate model is blocked for 60 s, within both limits,";
        self.hold_select(json!({}), HOLD_SECOND, what).await?;
        Ok(())
    }

    async fn hold_account_for_applies_recorded_cap(&self) -> Check {
        self.ladder(&[HOLD_FIRST, HOLD_PRICED]).await?;
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now_ms() + 300_000))
            .await?;
        let what = "a select with hold_wait_max_s 600 whose first ladder entry resets in 300 s";
        let answer = self
            .hold_select(json!({"hold_wait_max_s": 600}), HOLD_FIRST, what)
            .await?;
        // The window now ends further away: within the runner's own cap and
        // max_hold, beyond the cap the select recorded.
        self.block_provider(fixture::SUBSCRIPTION_PROVIDER, Some(now_ms() + 700_000))
            .await?;
        match self.hold_account_for(&answer, "undecided-admission", false).await? {
            Err(refusal) if refusal.code == RefusalCode::AccountHold => {}
            Err(refusal) => {
                return Err(format!(
                    "account_for without a decision, on accounts blocked for 700 s within max_hold 900 s, was refused {}, not account_hold",
                    refusal.code
                ))
            }
            Ok(_) => {
                return Err("account_for without a decision was answered, though every account is blocked".into())
            }
        }
        match self.hold_account_for(&answer, "decided-admission", true).await? {
            Err(refusal) if refusal.code == RefusalCode::AccountHold => Err(
                "account_for naming a decision whose select recorded hold_wait_max_s 600 was refused account_hold for a 700 s wait".into(),
            ),
            Err(_) => Ok(()),
            Ok(_) => Err("account_for naming the decision was answered, though every account is blocked".into()),
        }
    }
}
