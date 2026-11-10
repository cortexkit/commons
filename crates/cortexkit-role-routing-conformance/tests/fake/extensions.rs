use super::*;
use cortexkit_role_routing_conformance::wire::{
    budget::{self, BudgetSeat},
    observe::parse_account_observe_request_serving,
    panel::{
        self, PanelPolicy, PanelReplacement, PanelReplacementRequest, PanelSeat, PanelShortfall,
        PanelState, SelectPanelAnswer, SelectPanelRequest,
    },
};

pub(super) const GROUPS: &[&str] = &[
    "select_panel",
    "budget_quote",
    "explain",
    "account_status",
    "head_choices",
    "select_pin_slot",
    "select_available_routes",
    "select_prefer_prompt_cache",
    "select_settings",
    "observe_backoff",
    "observe_credential_refused",
    "report_outcomes",
    "report_usage_run_id",
    "consumer_models",
];
pub(super) const OPS: &[&str] = &[
    ops::ROUTE_SELECT_PANEL,
    ops::ROUTE_BUDGET_QUOTE,
    ops::ROUTE_EXPLAIN,
    ops::ROUTE_ACCOUNT_STATUS,
    ops::ROUTE_HEAD_CHOICES,
    ops::ROUTE_CONSUMER_MODELS,
];

impl FakeRoute {
    pub(super) fn fault(&self, check: &str) -> bool {
        self.defects.extension == Some(check)
    }

    pub(super) fn panel(&self, state: &mut State, params: &Value) -> Result<Value, Refusal> {
        if params.get("replace").is_some() {
            let replace: PanelReplacementRequest = serde_json::from_value(params.clone()).unwrap();
            let (owner, mut panel) = state
                .panels
                .get(&replace.replace.panel_id)
                .cloned()
                .ok_or_else(Refusal::unknown_decision)?;
            if owner != self.principal() && !self.fault("panel_foreign_replacement_unknown") {
                return Err(Refusal::unknown_decision());
            }
            let mut slot = replace.replace.slot;
            if slot >= panel.policy().size {
                if !self.fault("panel_replacement_invalid_slot") {
                    return Err(Refusal::invalid_request("replace.slot", "outside panel"));
                }
                slot = 0;
            }
            let phase = if slot < panel.policy().top {
                "top"
            } else {
                "extra"
            };
            let mut excluded = panel
                .seated_route_keys()
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            excluded.extend(replace.exclude.clone().unwrap_or_default());
            let families = panel
                .seats()
                .iter()
                .filter(|(standing, _)| **standing != slot)
                .map(|(_, seat)| seat.family.clone())
                .collect::<Vec<_>>();
            let catalog = Self::catalog(state);
            let candidate = catalog
                .models
                .iter()
                .find(|m| !excluded.contains(&m.route_key()) && !families.contains(&m.family));
            let mut replacement = if let Some(model) = candidate {
                let mut seat_request =
                    serde_json::to_value(panel.request().seat_request(&excluded)).unwrap();
                seat_request["pin"] = json!(model.route_key());
                let mut selected: SelectAnswer =
                    serde_json::from_value(self.select_extensions(state, &seat_request)?).unwrap();
                selected.pin_outcome = None;
                let temperature = panel
                    .request()
                    .temperatures
                    .as_ref()
                    .and_then(|t| t.get(slot as usize))
                    .copied();
                let mut seat: PanelSeat =
                    serde_json::from_value(json!({"slot":slot,"phase":phase,"answer":selected,
                    "family":model.family,"temperature":temperature}))
                    .unwrap();
                if self.fault("panel_replacement_preserves_slot") {
                    seat.temperature = Some(1.9);
                }
                PanelReplacement::seated(panel.panel_id(), seat)
            } else {
                PanelReplacement::unseated(
                    panel.panel_id(),
                    slot,
                    panel::ReplacementCause::parse("no_candidate"),
                )
            };
            if self.fault("panel_replacement_detail_requires_cause") {
                replacement.detail = Some(json!({"message":"No spare route", "causes":[]}));
            }
            panel.apply(&replacement);
            state.panels.insert(panel.panel_id().into(), (owner, panel));
            return to_value(replacement);
        }
        let request: SelectPanelRequest = serde_json::from_value(params.clone()).unwrap();
        let (size, top, extra) = state.panel_policy.unwrap();
        let mut catalog = Self::catalog(state);
        let generation = self
            .panel_calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if self.fault("panel_same_seed") && generation % 2 == 1 {
            catalog.models.rotate_left(1);
        }
        let mut families = Vec::new();
        let mut seats = Vec::<PanelSeat>::new();
        for model in &catalog.models {
            if seats.len() == size as usize {
                break;
            }
            if families.contains(&model.family) {
                continue;
            }
            let slot = seats.len();
            let mut seat_request = serde_json::to_value(request.seat_request(&[])).unwrap();
            seat_request["pin"] = json!(model.route_key());
            let mut selected: SelectAnswer =
                serde_json::from_value(self.select_extensions(state, &seat_request)?).unwrap();
            selected.pin_outcome = None;
            let seat = serde_json::from_value(json!({"slot":slot,"phase":if slot < top as usize { "top" } else { "extra" },
                "answer":selected,"family":model.family,"temperature":request.temperatures.as_ref().and_then(|t| t.get(slot))})).unwrap();
            seats.push(seat);
            families.push(model.family.clone());
        }
        if self.fault("panel_full") {
            seats.pop();
        }
        if self.fault("panel_family_and_route_unique") && seats.len() > 1 {
            seats[1].family = seats[0].family.clone();
        }
        if self.fault("panel_temperatures_by_slot") {
            seats[0].temperature = Some(1.9);
        }
        let mut answer = SelectPanelAnswer::new(
            state.next("panel"),
            PanelPolicy::new(size.into(), top.into(), extra.into()),
            seats,
        );
        if answer.seats.len() < size as usize && !self.fault("panel_shortfall") {
            answer.shortfall = Some(PanelShortfall::new(panel::ShortfallCode::parse(
                "diversity_unmeetable",
            )));
        }
        state.panels.insert(
            answer.panel_id.clone(),
            (
                self.principal().into(),
                PanelState::new(request, answer.clone()),
            ),
        );
        to_value(answer)
    }

    pub(super) fn select_extensions(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        let mut params = params.clone();
        let fields = [
            "pin_slot",
            "available_routes",
            "prefer_prompt_cache",
            "settings",
        ];
        if fields.iter().any(|f| params.get(*f).is_some()) {
            let mut groups = GROUPS.to_vec();
            groups.extend([
                SELECT_HARNESS,
                SELECT_HOLD_CONTROL,
                SELECT_PREFER,
                SELECT_EXCLUDE_PROVIDERS,
                SELECT_TASK_REF,
                "service_tier",
            ]);
            if self.fault("settings_unknown_member_refused")
                && params.pointer("/settings/unknown").is_some()
            {
                params["settings"] = json!({});
            }
            if self.fault("available_routes_empty_refused")
                && params.get("available_routes") == Some(&json!([]))
            {
                params["available_routes"] = json!([Self::catalog(state).models[0].route_key()]);
            }
            let accepts_undeclared = self
                .defects
                .extension
                .is_some_and(|f| f.starts_with("undeclared_"));
            parse_select_request_serving(
                &params,
                if self.extensions || accepts_undeclared {
                    &groups
                } else {
                    &[]
                },
            )?;
        }
        let slot = params
            .get("pin_slot")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(slot) = &slot {
            let key = state.pin_slots.get(slot).cloned();
            if self.fault("pin_slot_unknown_unavailable") && key.is_none() {
                params["pin"] = json!(Self::catalog(state).models[0].route_key());
            } else if let Some(key) = key {
                params["pin"] = json!(if self.fault("pin_slot_configured_pinned") {
                    Self::catalog(state).models[0].route_key()
                } else {
                    key
                });
            } else if self.extensions {
                return Err(Refusal::with(
                    RefusalCode::PinUnavailable,
                    "unknown slot",
                    &json!({"pin":slot,"reason":"unknown_slot"}),
                ));
            }
        }
        let available = params
            .get("available_routes")
            .and_then(Value::as_array)
            .cloned();
        let project = params.pointer("/settings/project_id").is_some();
        let prompt_cache = params.get("prefer_prompt_cache").is_some();
        for field in fields {
            params.as_object_mut().unwrap().remove(field);
        }
        if self.fault("prefer_prompt_cache_accepted") && prompt_cache {
            return Err(Refusal::invalid_request(
                "prefer_prompt_cache",
                "unsupported",
            ));
        }
        let original_catalog = state.catalog.clone();
        if let Some(routes) = available {
            if !self.fault("available_routes_honoured") {
                state
                    .catalog
                    .as_mut()
                    .unwrap()
                    .models
                    .retain(|model| routes.contains(&json!(model.route_key())));
            }
        }
        let result = self.select(state, &params);
        state.catalog = original_catalog;
        let mut selected = result?;
        if project {
            selected["project_settings"] = json!(if self.fault("settings_unresolved_selects") {
                "none"
            } else {
                "unresolved"
            });
        }
        state.extension_answers.insert(
            selected["decision_id"].as_str().unwrap().into(),
            selected.clone(),
        );
        Ok(selected)
    }

    pub(super) fn observe_extensions(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        let mut params = params.clone();
        if self.fault("credential_refused_retry_at_ms_refused")
            && params["kind"] == "credential_refused"
        {
            params.as_object_mut().unwrap().remove("retry_at_ms");
        }
        if self.fault("backoff_missing_retry_refused") && params["kind"] == "transient_backoff" {
            params["retry_at_ms"] = json!(now_ms() + 1000);
        }
        let accepts_undeclared = self
            .defects
            .extension
            .is_some_and(|f| f.starts_with("undeclared_observe_"));
        if accepts_undeclared {
            for field in ["decision_id", "retry_at_ms", "provider_code"] {
                params.as_object_mut().unwrap().remove(field);
            }
        } else {
            parse_account_observe_request_serving(
                &params,
                if self.extensions { GROUPS } else { &[] },
            )?;
        }
        if self.fault("backoff_with_admission_accepted") && params["kind"] == "transient_backoff" {
            return Err(Refusal::invalid_request("kind", "no backoff"));
        }
        for field in ["decision_id", "retry_at_ms", "provider_code"] {
            params.as_object_mut().unwrap().remove(field);
        }
        let answer = self.observe(state, &params)?;
        if self.extensions
            && params["kind"] == "credential_refused"
            && !self.fault("credential_refused_blocks_credential")
            && answer["observation"]["duplicate"] == false
        {
            let credential = params["credential_id"]
                .as_str()
                .map(String::from)
                .or_else(|| {
                    let admission = params["admission_id"].as_str()?;
                    state
                        .admissions
                        .get(&(self.principal().to_owned(), admission.to_owned()))
                        .map(|(_, answer)| answer.account.credential_id.clone())
                });
            if let Some(credential) = credential {
                state.ineligible.push((credential, None));
            }
        }
        Ok(answer)
    }

    pub(super) fn extension_op(
        &self,
        state: &mut State,
        op: &str,
        params: &Value,
    ) -> Result<Value, Refusal> {
        match op {
            ops::ROUTE_ACCOUNT_STATUS => {
                let account = params["account_id"].as_str().unwrap();
                let blocked = state.blocks.get(account);
                let mut value = json!({"provider":params["provider"], "account_id":account,
                    "limit_state":if blocked.is_some() { "limited" } else { "unknown" }});
                if let Some(Some(reset)) = blocked {
                    value["reset_at_ms"] = json!(reset);
                }
                if (self.fault("account_status_blocked_limited") && blocked.is_some())
                    || (self.fault("account_status_unknown") && blocked.is_none())
                {
                    value["limit_state"] = json!("available");
                    value.as_object_mut().unwrap().remove("reset_at_ms");
                }
                Ok(value)
            }
            ops::ROUTE_HEAD_CHOICES => {
                let request = cortexkit_role_routing_conformance::wire::head_choices::parse_head_choices_request(params)?;
                let catalog = Self::catalog(state);
                let runner = catalog
                    .runners
                    .iter()
                    .find(|r| json!(r.module) == params["runner_module"]);
                self.check_harness(runner.into_iter(), request.harness.as_deref())?;
                let mut models = catalog.models.iter().filter(|m| runner.is_some_and(|r|
                     (r.starts_sessions || self.fault("head_choices_non_starting_empty")) &&
                     !(self.fault("harness_limits_baseline_omits_excluded") && m.model == "model-a") &&
                     (self.fault("harness_limits_head_choices_ignored") || self.runner_accepts_model(state, &r.module, m, request.harness.as_deref())) &&
                     (r.route_keys.contains(&m.route_key()) || self.fault("head_choices_runner_routes"))))
                    .map(|m| json!({"provider":m.provider,"model":m.model,"variants":[],"service_tiers":[]})).collect::<Vec<_>>();
                models.sort_by_key(|m| {
                    (
                        m["provider"].as_str().unwrap().to_owned(),
                        m["model"].as_str().unwrap().to_owned(),
                    )
                });
                if self.fault("head_choices_valid") {
                    if let Some(first) = models.first().cloned() {
                        models.push(first);
                    }
                }
                if self.fault("head_choices_billing_valid") && !models.is_empty() {
                    models[0]["billing"] = json!("free");
                }
                Ok(
                    json!({"runner_module":params["runner_module"],"catalog_version":"fixture-v1","models":models}),
                )
            }
            ops::ROUTE_CONSUMER_MODELS => {
                let request = cortexkit_role_routing_conformance::wire::consumer_models::parse_consumer_models_request(params)?;
                if let Some((owner, _)) = state.declarations.get(&request.consumer) {
                    let scope_owner = self.stamp.scope.as_ref().map(|scope| scope.owner.as_str());
                    if !owner.as_deref().is_some_and(|owner| {
                        self.principal() == owner || scope_owner == Some(owner)
                    }) {
                        return Err(Refusal::consumer_not_owned(request.consumer));
                    }
                } else if !state.consumers.iter().any(|c| c.id == request.consumer)
                    && !self.fault("consumer_models_unknown_consumer")
                {
                    return Err(Refusal::unknown_consumer(request.consumer));
                }
                let mut models = Self::catalog(state).models.iter().map(|m| json!({"provider":m.provider,"model":m.model,"elo":1200.5,"eq":0.25,"excluded":false,"billing":"per_token"})).collect::<Vec<_>>();
                models.sort_by_key(|m| {
                    (
                        m["provider"].as_str().unwrap().to_owned(),
                        m["model"].as_str().unwrap().to_owned(),
                    )
                });
                let consumer = if self.fault("consumer_models_valid") {
                    "wrong-consumer"
                } else {
                    &request.consumer
                };
                Ok(json!({"catalog_version":"fixture-v1","consumer":consumer,"models":models}))
            }
            ops::ROUTE_BUDGET_QUOTE | ops::ROUTE_EXPLAIN => {
                let foreign_fault = (self.fault("budget_quote_foreign_unknown")
                    && op == ops::ROUTE_BUDGET_QUOTE)
                    || (self.fault("explain_foreign_unknown") && op == ops::ROUTE_EXPLAIN);
                let ids = if op == ops::ROUTE_EXPLAIN {
                    vec![params["decision_id"].as_str().unwrap()]
                } else {
                    params["decisions"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|v| v.as_str().unwrap())
                        .collect()
                };
                let mut answers = Vec::new();
                for id in ids {
                    let decision = state
                        .decisions
                        .get(id)
                        .ok_or_else(Refusal::unknown_decision)?;
                    if decision.principal != self.principal() && !foreign_fault {
                        return Err(Refusal::unknown_decision());
                    }
                    answers.push(
                        serde_json::from_value::<SelectAnswer>(state.extension_answers[id].clone())
                            .unwrap(),
                    );
                }
                if op == ops::ROUTE_BUDGET_QUOTE {
                    let seats = answers
                        .iter()
                        .map(BudgetSeat::from_select_answer)
                        .collect::<Vec<_>>();
                    let mut value = to_value(budget::quote(
                        &seats,
                        params.get("prompt_tokens").and_then(Value::as_u64),
                    )?)?;
                    if self.fault("budget_quote_matches_helper") {
                        value["package"]["tokens"] = json!(0);
                    }
                    Ok(value)
                } else {
                    let a = &answers[0];
                    let mut value = json!({"decision_id":a.decision_id,"route":a.route,"runner":a.runner,
                        "serving":{"provider":a.route.provider},"policy_version":"fixture-v1","reason_code":"ranked"});
                    if self.fault("explain_matches_selection") {
                        value["serving"]["provider"] = json!("wrong-provider");
                    }
                    Ok(value)
                }
            }
            _ => unreachable!(),
        }
    }
}
