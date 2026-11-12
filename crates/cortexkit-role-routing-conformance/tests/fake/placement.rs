use super::*;
use cortexkit_role_routing_conformance::wire::{
    capabilities,
    placement::Placement,
    price::{Price, RatesPerMtok},
};

pub(super) const GROUPS: &[&str] = &[
    capabilities::CONSUMER_DECLARATIONS,
    capabilities::RUNNER_HARNESS_FACTS,
    capabilities::DECISION_HARNESS,
    capabilities::SELECT_KIND,
    capabilities::SELECT_ELIGIBLE_ROUTES,
    capabilities::SELECT_PRICE,
    capabilities::REPORT_ATTEMPT_OUTCOMES,
];

fn divided(rates: &RatesPerMtok, multiplier: f64) -> RatesPerMtok {
    let divide = |n: &serde_json::Number| {
        serde_json::Number::from_f64(n.as_f64().unwrap() / multiplier).unwrap()
    };
    let mut rates = rates.clone();
    rates.input = divide(&rates.input);
    rates.output = divide(&rates.output);
    rates.cache_read = rates.cache_read.as_ref().map(divide);
    rates.cache_write = rates.cache_write.as_ref().map(divide);
    rates.cache_write_ttl = rates.cache_write_ttl.as_ref().map(|map| {
        map.iter()
            .map(|(ttl, n)| (ttl.clone(), divide(n)))
            .collect()
    });
    rates
}

impl FakeRoute {
    pub(super) fn select_placement(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        if !self.policy_control {
            return self.select_extensions(state, params);
        }
        let mut params = params.clone();
        let consumer = params
            .get("consumer")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let inherited = state
            .declarations
            .iter()
            .filter(|(name, _)| {
                consumer == **name
                    || consumer
                        .strip_prefix(name.as_str())
                        .is_some_and(|tail| tail.starts_with('.'))
            })
            .max_by_key(|(name, _)| name.len())
            .map(|(_, value)| value.clone());
        if let Some(inherited) = inherited {
            state
                .declarations
                .entry(consumer.clone())
                .or_insert(inherited);
        }
        let defaults = state
            .declarations
            .get(&consumer)
            .map(|(_, defaults)| defaults.clone())
            .unwrap_or_default();
        if params
            .pointer("/requirements/iq")
            .is_none_or(Value::is_null)
        {
            if let Some(iq) = defaults.iq {
                params["requirements"]["iq"] = json!(iq);
            }
        }
        let requested =
            cortexkit_role_routing_conformance::wire::select::parse_select_request_serving(
                &params,
                capabilities::KNOWN,
            )?;
        let mut catalog = Self::catalog(state);
        let original_catalog = state.catalog.clone();
        let original_ladders = state.ladders.clone();
        if let Some(runners) = &requested.runners {
            catalog
                .runners
                .retain(|runner| runners.contains(&runner.module));
        }
        let kind = requested
            .kind
            .as_ref()
            .or(defaults.kind.as_ref())
            .map_or("llm", |kind| kind.as_str());
        if !self.fault("placement_select_kind") {
            catalog.models.retain(|model| {
                state
                    .selection_facts
                    .kinds
                    .get(&model.route_key())
                    .map_or("llm", String::as_str)
                    == kind
            });
        }
        if !self.fault("placement_eligible_routes") {
            if let Some(routes) = &requested.eligible_routes {
                catalog.models.retain(|model| {
                    routes
                        .iter()
                        .any(|route| route.route_key() == model.route_key())
                });
                if routes.is_empty() {
                    return Err(Refusal::no_route(NoRouteReason::AllInadequate, None));
                }
            }
        }
        if !self.fault("placement_consumer_declarations") {
            let placement = match &defaults.placement {
                Some(Placement::Module { module }) => Some(module.clone()),
                Some(Placement::CallerRunner(_)) => state.selection_facts.caller_runner.clone(),
                _ => None,
            };
            if let Some(module) = placement {
                catalog.runners.retain(|runner| runner.module == module);
            }
            if let Some(ladder) = &defaults.ladder {
                state
                    .ladders
                    .entry(consumer.clone())
                    .or_insert(ladder.clone());
            }
        }
        let mut harnesses = std::collections::BTreeMap::new();
        for runner in &mut catalog.runners {
            let declaration = state.runner_declarations.get(&runner.module);
            runner.route_keys.retain(|key| {
                let offered = requested
                    .harness
                    .clone()
                    .map(|h| vec![h])
                    .or_else(|| runner.harnesses.clone())
                    .unwrap_or_default();
                let mut offered = offered;
                if let Some(default) = &runner.default_harness {
                    offered.sort_by_key(|h| h != default);
                }
                let chosen = offered.into_iter().find(|harness| {
                    if self.fault("placement_harness_facts") {
                        return true;
                    }
                    declaration.is_none_or(|decl| {
                        let limits = decl.limits_for_harness(Some(harness)).unwrap_or_default();
                        limits.starts_sessions.unwrap_or(decl.starts_sessions)
                            && !limits
                                .exclude
                                .as_ref()
                                .and_then(|specs| {
                                    cortexkit_role_routing_conformance::wire::route_spec::resolve(
                                        specs.iter().map(String::as_str),
                                        key,
                                        true,
                                    )
                                })
                                .unwrap_or(false)
                    })
                });
                if let Some(harness) = chosen {
                    harnesses.insert((runner.module.clone(), key.clone()), harness);
                    true
                } else {
                    declaration.is_none()
                }
            });
        }
        if requested.harness.is_some()
            && catalog
                .runners
                .iter()
                .all(|runner| runner.route_keys.is_empty())
        {
            state.ladders = original_ladders;
            return Err(Refusal::no_route(NoRouteReason::HarnessUnavailable, None));
        }
        for field in ["kind", "eligible_routes"] {
            params.as_object_mut().unwrap().remove(field);
        }
        state.catalog = Some(catalog);
        let result = self.select_extensions(state, &params);
        state.catalog = original_catalog;
        state.ladders = original_ladders;
        let mut answer = result?;
        let key = format!(
            "{}/{}",
            answer["route"]["provider"].as_str().unwrap(),
            answer["route"]["model"].as_str().unwrap()
        );
        let runner = answer["runner"]["module"].as_str().unwrap().to_owned();
        if let Some(harness) = harnesses.get(&(runner, key.clone())) {
            answer["runner"]["harness"] = json!(harness);
        }
        if self.fault("placement_decision_harness") {
            answer["runner"].as_object_mut().unwrap().remove("harness");
        }
        if let Some(rates) = state.selection_facts.list_rates.get(&key) {
            let mut price = match state.selection_facts.multipliers.get(&key) {
                Some(multiplier) => Price::subscription_equivalent(
                    divided(rates, multiplier.as_f64().unwrap()),
                    multiplier.clone(),
                ),
                None => Price::metered(rates.clone()),
            };
            price.estimated_task_usd = state.selection_facts.estimates.get(&key).cloned();
            if self.fault("placement_select_price") {
                price.multiplier = Some(0.into());
            }
            answer["price"] = json!(price);
        }
        Ok(answer)
    }
}
