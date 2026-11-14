use super::*;
use crate::wire::{
    account_status::{check_account_status, AccountStatusAnswer, AccountStatusRequest},
    budget::{check_budget_quote, BudgetQuote, BudgetSeat},
    consumer_models::{check_consumer_models, ConsumerModelsAnswer, ConsumerModelsRequest},
    explain::{check_explain, ExplainAnswer, ExplainRequest},
    head_choices::{check_head_choices, HeadChoicesAnswer, HeadChoicesRequest},
    panel::{
        check_panel_answer, check_panel_replacement, PanelReplacement, PanelReplacementRequest,
        PanelState, SelectPanelAnswer, SelectPanelRequest,
    },
};

/// (check name, capability group, true for an undeclared-member refusal).
pub(super) const CASES: &[(&str, &str, bool)] = &[
    ("panel_full", "select_panel", false),
    ("panel_shortfall", "select_panel", false),
    ("panel_family_and_route_unique", "select_panel", false),
    ("panel_temperatures_by_slot", "select_panel", false),
    ("panel_same_seed", "select_panel", false),
    ("panel_replacement_preserves_slot", "select_panel", false),
    (
        "panel_replacement_detail_requires_cause",
        "select_panel",
        false,
    ),
    ("panel_foreign_replacement_unknown", "select_panel", false),
    ("panel_replacement_invalid_slot", "select_panel", false),
    ("budget_quote_matches_helper", "budget_quote", false),
    ("budget_quote_foreign_unknown", "budget_quote", false),
    ("explain_matches_selection", "explain", false),
    ("explain_foreign_unknown", "explain", false),
    ("account_status_blocked_limited", "account_status", false),
    ("account_status_unknown", "account_status", false),
    ("head_choices_valid", "head_choices", false),
    ("head_choices_runner_routes", "head_choices", false),
    ("head_choices_non_starting_empty", "head_choices", false),
    ("head_choices_billing_valid", "head_choices", false),
    ("consumer_models_valid", "consumer_models", false),
    ("consumer_models_unknown_consumer", "consumer_models", false),
    (
        "report_usage_run_id_repeat_accepted",
        "report_usage_run_id",
        false,
    ),
    (
        "report_usage_run_id_overlong_refused",
        "report_usage_run_id",
        false,
    ),
    ("pin_slot_configured_pinned", "select_pin_slot", false),
    ("pin_slot_unknown_unavailable", "select_pin_slot", false),
    ("undeclared_pin_slot_refused", "select_pin_slot", true),
    (
        "available_routes_honoured",
        "select_available_routes",
        false,
    ),
    (
        "available_routes_empty_refused",
        "select_available_routes",
        false,
    ),
    (
        "undeclared_available_routes_refused",
        "select_available_routes",
        true,
    ),
    (
        "prefer_prompt_cache_accepted",
        "select_prefer_prompt_cache",
        false,
    ),
    (
        "undeclared_prefer_prompt_cache_refused",
        "select_prefer_prompt_cache",
        true,
    ),
    ("settings_unresolved_selects", "select_settings", false),
    ("settings_unknown_member_refused", "select_settings", false),
    ("undeclared_settings_refused", "select_settings", true),
    ("backoff_with_admission_accepted", "observe_backoff", false),
    ("backoff_missing_retry_refused", "observe_backoff", false),
    (
        "observe_credential_refused_requires_backoff",
        "observe_credential_refused",
        false,
    ),
    (
        "credential_refused_blocks_credential",
        "observe_credential_refused",
        false,
    ),
    (
        "credential_refused_retry_at_ms_refused",
        "observe_credential_refused",
        false,
    ),
    (
        "undeclared_observe_decision_id_refused",
        "observe_backoff",
        true,
    ),
    (
        "undeclared_observe_retry_at_ms_refused",
        "observe_backoff",
        true,
    ),
    (
        "undeclared_observe_provider_code_refused",
        "observe_backoff",
        true,
    ),
];

// Compare only deterministic, decision-bearing members. Opaque ids, expiry,
// prose and informative policy members may legitimately change after reset.
fn seeded_members(panel: &SelectPanelAnswer) -> Value {
    json!({
        "policy": {"size":panel.policy.size, "top":panel.policy.top, "extra":panel.policy.extra},
        "shortfall": panel.shortfall.as_ref().map(|s| &s.code),
        "seats": panel.seats.iter().map(|seat| json!({
            "slot":seat.slot, "phase":seat.phase, "route":seat.answer.route,
            "runner":seat.answer.runner.module, "family":seat.family, "temperature":seat.temperature
        })).collect::<Vec<_>>()
    })
}

impl<S: RoutingSubject> Suite<'_, S> {
    pub(super) async fn extension_check(&self, name: &str) -> Result<Ending, String> {
        let (_, group, undeclared) = CASES.iter().find(|(case, _, _)| *case == name).unwrap();
        let route = self.route_as(CONSUMER).await?;
        let describe: RoleDescribe =
            answer(&route, ops::ROLE_DESCRIBE, &json!({}), "describe").await?;
        if describe.serves(group) == *undeclared {
            return Ok(Ending::NotApplicable(format!(
                "role.describe {} {group}",
                if *undeclared {
                    "declares"
                } else {
                    "does not declare"
                }
            )));
        }
        if name == "observe_credential_refused_requires_backoff" {
            cortexkit_role_routing::describe::check_describe(&describe)
                .map_err(|problem| format!("role.describe: {problem}"))?;
        } else if name.starts_with("credential_refused_") {
            self.credential_refused_check(name).await?;
        } else if name.starts_with("panel_") {
            self.panel_check(name, &route).await?;
        } else if name.starts_with("budget_quote_") || name.starts_with("explain_") {
            let selected = select(&route, &base(), "decision fixture").await?;
            let mut decisions = vec![selected.clone()];
            if name.starts_with("budget_quote_") {
                decisions.push(
                    select(
                        &route,
                        &with(
                            base(),
                            "pin",
                            json!(another_route_key(&selected.route.route_key())?),
                        ),
                        "second budget decision",
                    )
                    .await?,
                );
            }
            let op = if name.starts_with("budget") {
                ops::ROUTE_BUDGET_QUOTE
            } else {
                ops::ROUTE_EXPLAIN
            };
            let params = if name.starts_with("budget") {
                json!({"decisions":decisions.iter().map(|d| &d.decision_id).collect::<Vec<_>>(), "prompt_tokens":180_000})
            } else {
                json!({"decision_id":selected.decision_id})
            };
            if name.ends_with("foreign_unknown") {
                let foreign = self.route_as(fixture::OTHER_CONSUMER).await?;
                refused(
                    &foreign,
                    op,
                    &params,
                    RefusalCode::UnknownDecision,
                    "foreign decision",
                )
                .await?;
            } else if name.starts_with("budget") {
                let quote: BudgetQuote = answer(&route, op, &params, "budget quote").await?;
                check_budget_quote(
                    &decisions
                        .iter()
                        .map(BudgetSeat::from_select_answer)
                        .collect::<Vec<_>>(),
                    Some(180_000),
                    &quote,
                )
                .map_err(|e| e.to_string())?;
            } else {
                let explained: ExplainAnswer = answer(&route, op, &params, "explain").await?;
                check_explain(
                    &selected,
                    &ExplainRequest::new(&selected.decision_id),
                    &explained,
                )
                .map_err(|e| e.to_string())?;
            }
        } else if name.starts_with("account_status_") {
            let blocked = name == "account_status_blocked_limited";
            let reset = now_ms() + 60_000;
            let account = if blocked {
                "account-1"
            } else {
                "unknown-account"
            };
            if blocked {
                self.subject.block_account(account, Some(reset)).await?;
            }
            let params = json!({"provider":fixture::SUBSCRIPTION_PROVIDER, "account_id":account});
            let status: AccountStatusAnswer =
                answer(&route, ops::ROUTE_ACCOUNT_STATUS, &params, "account status").await?;
            check_account_status(
                &AccountStatusRequest::new(fixture::SUBSCRIPTION_PROVIDER, account),
                &status,
            )
            .map_err(|e| e.to_string())?;
            if status.limit_state.as_str() != if blocked { "limited" } else { "unknown" }
                || status.reset_at_ms != if blocked { Some(reset) } else { None }
            {
                return Err(format!("unexpected account status: {status:?}"));
            }
        } else if name.starts_with("consumer_models_") {
            if name == "consumer_models_unknown_consumer" {
                let refusal = refused(
                    &route,
                    ops::ROUTE_CONSUMER_MODELS,
                    &json!({"consumer":fixture::UNCONFIGURED}),
                    RefusalCode::UnknownConsumer,
                    "unknown consumer models",
                )
                .await?;
                if refusal.detail.get("consumer") != Some(&json!(fixture::UNCONFIGURED)) {
                    return Err("unknown_consumer must echo consumer".into());
                }
            } else {
                let request = ConsumerModelsRequest::new(SUMMARY);
                let models: ConsumerModelsAnswer = answer(
                    &route,
                    ops::ROUTE_CONSUMER_MODELS,
                    &serde_json::to_value(&request).unwrap(),
                    "consumer models",
                )
                .await?;
                check_consumer_models(&request, &models).map_err(|e| e.to_string())?;
            }
        } else if name.starts_with("report_usage_run_id_") {
            let selected = select(&route, &base(), "usage decision").await?;
            let mut params = json!({"decision_id":selected.decision_id,"outcome":"completed","usage":{"run_id":"run-1","input_tokens":10}});
            if name == "report_usage_run_id_overlong_refused" {
                params["usage"]["run_id"] = json!("x".repeat(129));
                Self::invalid_field(&route, ops::ROUTE_REPORT, &params, "usage.run_id").await?;
            } else {
                let _: ReportAnswer =
                    answer(&route, ops::ROUTE_REPORT, &params, "first run usage").await?;
                params["usage"]["run_id"] = json!("run-2");
                let _: ReportAnswer =
                    answer(&route, ops::ROUTE_REPORT, &params, "second run usage").await?;
                params["usage"]["run_id"] = json!("run-1");
                params["usage"]["input_tokens"] = json!(999);
                let _: ReportAnswer =
                    answer(&route, ops::ROUTE_REPORT, &params, "repeated run usage").await?;
            }
        } else if name.starts_with("head_choices_") {
            self.head_check(name, &route).await?;
        } else if name.contains("backoff") || name.starts_with("undeclared_observe_") {
            self.backoff_check(name).await?;
        } else {
            self.select_extension_check(name, &route).await?;
        }
        Ok(Ending::Passed)
    }

    async fn panel_fixture(&self, shortfall: bool) -> Result<(), String> {
        let mut catalog = fixture::panel_catalog();
        if shortfall {
            catalog.models.truncate(1);
            let keys = catalog
                .models
                .iter()
                .map(|m| m.route_key())
                .collect::<Vec<_>>();
            for runner in &mut catalog.runners {
                runner.route_keys = keys.clone();
            }
        }
        self.subject.configure_catalog(&catalog).await?;
        self.subject.configure_panel_class("audit", 3, 2, 1).await
    }

    async fn panel_check(&self, name: &str, route: &S::Route) -> Check {
        self.panel_fixture(name == "panel_shortfall").await?;
        let params = json!({"class":"audit", "consumer":SUMMARY, "seed":"repeatable", "temperatures":[0.2,1.4]});
        let request: SelectPanelRequest = serde_json::from_value(params.clone()).unwrap();
        let first: SelectPanelAnswer =
            answer(route, ops::ROUTE_SELECT_PANEL, &params, "panel").await?;
        check_panel_answer(&request, &first).map_err(|e| e.to_string())?;
        if first.policy.size != 3 || first.policy.top != 2 || first.policy.extra != 1 {
            return Err("panel did not use the configured class counts".into());
        }
        if name == "panel_shortfall" {
            if first.seats.len() != 1 || first.shortfall.is_none() {
                return Err("one eligible model must seat one member with a shortfall".into());
            }
        } else if first.seats.len() != 3 || first.shortfall.is_some() {
            return Err("enough eligible models must fill the configured panel".into());
        }
        for seat in &first.seats {
            let catalog = fixture::panel_catalog();
            let model = catalog
                .models
                .iter()
                .find(|m| m.route_key() == seat.answer.route.route_key())
                .ok_or("panel answered a route outside the fixture")?;
            if model.family != seat.family {
                return Err("panel did not carry the catalog family".into());
            }
        }
        match name {
            "panel_same_seed" => {
                self.fresh().await?;
                self.panel_fixture(false).await?;
                let fresh_route = self.route_as(CONSUMER).await?;
                let second: SelectPanelAnswer = answer(
                    &fresh_route,
                    ops::ROUTE_SELECT_PANEL,
                    &params,
                    "seeded panel after reset",
                )
                .await?;
                check_panel_answer(&request, &second).map_err(|e| e.to_string())?;
                if seeded_members(&first) != seeded_members(&second) {
                    return Err("same seed changed the panel".into());
                }
            }
            "panel_replacement_preserves_slot" | "panel_replacement_detail_requires_cause" => {
                // Exercise both phases, including a temperature absent beyond the list.
                for slot in [0, 1, 2] {
                    self.fresh().await?;
                    self.panel_fixture(false).await?;
                    let fresh_route = self.route_as(CONSUMER).await?;
                    let panel: SelectPanelAnswer = answer(
                        &fresh_route,
                        ops::ROUTE_SELECT_PANEL,
                        &params,
                        "replacement fixture",
                    )
                    .await?;
                    check_panel_answer(&request, &panel).map_err(|e| e.to_string())?;
                    let replace = PanelReplacementRequest::new(&panel.panel_id, slot);
                    let state = PanelState::new(request.clone(), panel);
                    let replacement: PanelReplacement = answer(
                        &fresh_route,
                        ops::ROUTE_SELECT_PANEL,
                        &serde_json::to_value(&replace).unwrap(),
                        "replacement",
                    )
                    .await?;
                    check_panel_replacement(&state, &replace, &replacement)
                        .map_err(|e| e.to_string())?;
                    if replacement.seat.is_none() {
                        return Err("spare fixture families must allow a replacement".into());
                    }
                }
            }
            "panel_foreign_replacement_unknown" => {
                let foreign = self.route_as(fixture::OTHER_CONSUMER).await?;
                refused(
                    &foreign,
                    ops::ROUTE_SELECT_PANEL,
                    &json!({"replace":{"panel_id":first.panel_id,"slot":0}}),
                    RefusalCode::UnknownDecision,
                    "foreign panel",
                )
                .await?;
            }
            "panel_replacement_invalid_slot" => {
                Self::invalid_field(
                    route,
                    ops::ROUTE_SELECT_PANEL,
                    &json!({"replace":{"panel_id":first.panel_id,"slot":3}}),
                    "replace.slot",
                )
                .await?;
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) async fn invalid_field(
        route: &S::Route,
        op: &str,
        params: &Value,
        field: &str,
    ) -> Check {
        let refusal = refused(route, op, params, RefusalCode::InvalidRequest, field).await?;
        if refusal.detail.get("field") != Some(&json!(field)) {
            return Err(format!("refusal must name {field}: {refusal}"));
        }
        Ok(())
    }

    async fn head_check(&self, name: &str, route: &S::Route) -> Check {
        let mut catalog = fixture::catalog();
        // Restrict one runner so returning the whole catalog is observably wrong.
        if name == "head_choices_runner_routes" {
            catalog.runners[0].route_keys.truncate(1);
        }
        if name == "head_choices_non_starting_empty" {
            catalog.runners[0].starts_sessions = false;
        }
        self.subject.configure_catalog(&catalog).await?;
        let request = HeadChoicesRequest::new(fixture::RUNNER_ONE);
        let choices: HeadChoicesAnswer = answer(
            route,
            ops::ROUTE_HEAD_CHOICES,
            &serde_json::to_value(&request).unwrap(),
            "head choices",
        )
        .await?;
        check_head_choices(&request, &choices).map_err(|e| e.to_string())?;
        if name == "head_choices_non_starting_empty" && !choices.models.is_empty() {
            return Err("a non-starting runner must have empty models".into());
        }
        if choices.models.iter().any(|m| {
            !catalog.runners[0]
                .route_keys
                .contains(&format!("{}/{}", m.provider, m.model))
        }) {
            return Err("listed model is not among the named runner's route keys".into());
        }
        Ok(())
    }

    async fn select_extension_check(&self, name: &str, route: &S::Route) -> Check {
        let (field, value) = if name.contains("pin_slot") {
            (
                "pin_slot",
                json!(if name == "pin_slot_unknown_unavailable" {
                    "unknown-slot"
                } else {
                    "review"
                }),
            )
        } else if name.contains("available_routes") {
            let key = fixture::catalog().models.last().unwrap().route_key();
            (
                "available_routes",
                if name == "available_routes_empty_refused" {
                    json!([])
                } else {
                    json!([key])
                },
            )
        } else if name.contains("prefer_prompt_cache") {
            ("prefer_prompt_cache", json!(true))
        } else {
            (
                "settings",
                if name == "settings_unknown_member_refused" {
                    json!({"unknown":true})
                } else {
                    json!({"project_id":"never-configured-project"})
                },
            )
        };
        let params = with(base(), field, value);
        if name.starts_with("undeclared_") || name == "available_routes_empty_refused" {
            return Self::invalid_field(route, ops::ROUTE_SELECT, &params, field).await;
        }
        if name == "settings_unknown_member_refused" {
            return Self::invalid_field(route, ops::ROUTE_SELECT, &params, "settings.unknown")
                .await;
        }
        if name == "pin_slot_unknown_unavailable" {
            let refusal = refused(
                route,
                ops::ROUTE_SELECT,
                &params,
                RefusalCode::PinUnavailable,
                "unknown slot",
            )
            .await?;
            if refusal.detail.get("pin") != Some(&json!("unknown-slot"))
                || refusal.detail.get("reason") != Some(&json!("unknown_slot"))
            {
                return Err(format!(
                    "unknown slot refusal did not identify slot and reason: {refusal}"
                ));
            }
            return Ok(());
        }
        let pinned = fixture::catalog().models.last().unwrap().route_key();
        if field == "pin_slot" {
            self.subject.configure_pin_slot("review", &pinned).await?;
        }
        let selected = select(route, &params, name).await?;
        if field == "pin_slot"
            && (selected.route.route_key() != pinned
                || selected.pin_outcome.as_ref().map(|p| p.as_str()) != Some("pinned"))
        {
            return Err("configured slot must be pinned to its configured route key".into());
        }
        if name == "settings_unresolved_selects"
            && selected.project_settings.as_ref().map(|s| s.as_str()) != Some("unresolved")
        {
            return Err(
                "an unconfigured project must select with project_settings unresolved".into(),
            );
        }
        Ok(())
    }

    async fn credential_refused_check(&self, name: &str) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let admitted: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for("credential-wall", "credential-wall", "model-c"),
            "credential refusal admission",
        )
        .await?;
        let mut params = json!({"event_id": "credential-wall-event", "kind": "credential_refused",
            "admission_id": "credential-wall", "provider_code": "credit_exhausted"});
        if name == "credential_refused_retry_at_ms_refused" {
            params["retry_at_ms"] = json!(now_ms() + 30_000);
            return Self::invalid_field(
                &runner,
                ops::ROUTE_ACCOUNT_OBSERVE,
                &params,
                "retry_at_ms",
            )
            .await;
        }
        let _: AccountObserveAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_OBSERVE,
            &params,
            "credential refusal",
        )
        .await?;
        let _: AccountObserveAnswer = answer(&runner, ops::ROUTE_ACCOUNT_OBSERVE,
            &json!({"event_id": "credential-wall-finished", "kind": "run_finished", "admission_id": "credential-wall"}),
            "release refused credential admission").await?;
        // Probe only the refused credential, on another model and runner too,
        // so rotating keys or retaining the original pin cannot hide a missing block.
        for (index, principal) in [fixture::RUNNER_ONE, fixture::RUNNER_TWO]
            .iter()
            .enumerate()
        {
            let next_runner = self.route_as(principal).await?;
            let mut next = Self::account_for(
                &format!("after-credential-wall-{index}"),
                &format!("after-credential-wall-{index}"),
                "model-d",
            );
            next["candidates"] = json!({"view_digest": "refused-credential", "accounts": [{
                "account_id": admitted.account.account_id, "credential_id": admitted.account.credential_id
            }]});
            match call::<_, AccountForAnswer>(&next_runner, ops::ROUTE_ACCOUNT_FOR, &next).await? {
                Ok(after) if after.account.credential_id == admitted.account.credential_id => {
                    return Err(
                        "account_for admitted the credential after credential_refused".into(),
                    );
                }
                Ok(_) => {
                    return Err("account_for answered outside the sole credential candidate".into())
                }
                Err(refusal)
                    if matches!(
                        refusal.code,
                        RefusalCode::AuthRequired
                            | RefusalCode::AccountPoolExhausted
                            | RefusalCode::AccountHold
                    ) => {}
                Err(refusal) => {
                    return Err(format!(
                        "unexpected refusal after credential_refused: {refusal}"
                    ))
                }
            }
        }
        Ok(())
    }

    async fn backoff_check(&self, name: &str) -> Check {
        let runner = self.route_as(fixture::RUNNER_ONE).await?;
        let mut params = json!({"event_id":"backoff-event", "kind":"transient_backoff",
            "admission_id":"backoff-admission", "observed_at_ms":now_ms(), "retry_at_ms":now_ms()+30_000});
        if let Some(field) = name
            .strip_prefix("undeclared_observe_")
            .and_then(|s| s.strip_suffix("_refused"))
        {
            params = json!({"event_id":"undeclared-event", "kind":"run_finished", "observed_at_ms":now_ms()});
            params[field] = if field == "retry_at_ms" {
                json!(now_ms() + 30_000)
            } else {
                json!("evidence")
            };
            return Self::invalid_field(&runner, ops::ROUTE_ACCOUNT_OBSERVE, &params, field).await;
        }
        let admitted: AccountForAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_FOR,
            &Self::account_for("backoff-admission", "backoff-conversation", "model-c"),
            "backoff admission",
        )
        .await?;
        params["served_account_id"] = json!(admitted.account.account_id);
        params["provider_code"] = json!("retry_later");
        if name == "backoff_missing_retry_refused" {
            params.as_object_mut().unwrap().remove("retry_at_ms");
            return Self::invalid_field(
                &runner,
                ops::ROUTE_ACCOUNT_OBSERVE,
                &params,
                "retry_at_ms",
            )
            .await;
        }
        let _: AccountObserveAnswer = answer(
            &runner,
            ops::ROUTE_ACCOUNT_OBSERVE,
            &params,
            "transient backoff",
        )
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::{
        panel::{PanelPhase, PanelPolicy, PanelSeat},
        select::{RouteRef, RunnerRef},
    };

    fn panel() -> Value {
        let route = RouteRef::new("provider", "model");
        let answer = SelectAnswer::new(
            "decision",
            route.clone(),
            RunnerRef::new("runner", route),
            200_000,
            32_000,
            1000,
            "Selected.",
        );
        let mut seat = PanelSeat::new(0, PanelPhase::Top, answer);
        seat.family = Some("family".into());
        seat.temperature = Some(0.2);
        let mut value = serde_json::to_value(SelectPanelAnswer::new(
            "panel",
            PanelPolicy::new(3, 2, 1),
            vec![seat],
        ))
        .unwrap();
        value["shortfall"] = json!({"code":"size_reduced","detail":"reason"});
        for field in ["ranking", "extra_ranking", "floor", "seed"] {
            value["policy"][field] = json!("informative");
        }
        value["seats"][0]["answer"]["detail"] = json!({"note":"detail"});
        value
    }

    fn projection(value: Value) -> Value {
        seeded_members(&serde_json::from_value(value).unwrap())
    }

    #[test]
    fn routing_070_same_seed_compares_each_listed_member() {
        let original = panel();
        let expected = projection(original.clone());
        assert_eq!(
            expected,
            json!({"policy":{"size":3,"top":2,"extra":1},"shortfall":"size_reduced",
            "seats":[{"slot":0,"phase":"top","route":{"provider":"provider","model":"model"},
                "runner":"runner","family":"family","temperature":0.2}]})
        );
        for (path, value) in [
            ("/policy/size", json!(4)),
            ("/policy/top", json!(1)),
            ("/policy/extra", json!(2)),
            ("/shortfall/code", json!("floor_unmeetable")),
            ("/seats/0/slot", json!(1)),
            ("/seats/0/phase", json!("extra")),
            ("/seats/0/answer/route/provider", json!("other")),
            ("/seats/0/answer/route/model", json!("other")),
            ("/seats/0/answer/runner/module", json!("other")),
            ("/seats/0/family", json!("other")),
            ("/seats/0/temperature", json!(0.4)),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert_ne!(expected, projection(changed), "must compare {path}");
        }
        let mut variant = original.clone();
        variant["seats"][0]["answer"]["route"]["variant"] = json!("variant");
        assert_ne!(
            expected,
            projection(variant),
            "the full route includes its variant"
        );
    }

    #[test]
    fn routing_070_same_seed_ignores_each_listed_informative_member() {
        let original = panel();
        for (path, value) in [
            ("/panel_id", json!("different")),
            ("/seats/0/answer/decision_id", json!("different")),
            ("/seats/0/answer/expires_at_ms", json!(2000)),
            ("/seats/0/answer/explanation", json!("Different.")),
            ("/seats/0/answer/detail", json!({"different":true})),
            ("/shortfall/detail", json!("different")),
            ("/policy/ranking", json!([1])),
            ("/policy/extra_ranking", json!([2])),
            ("/policy/floor", json!({"other":true})),
            ("/policy/seed", json!(null)),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert_eq!(
                projection(original.clone()),
                projection(changed),
                "must ignore {path}"
            );
        }
    }
}
