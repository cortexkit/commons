use super::*;
use cortexkit_role_routing_conformance::{
    fixture,
    wire::{capabilities, select_engine as engine, select_media as media, select_search as search},
    SearchRouteFacts,
};

#[derive(Default)]
pub(super) struct SearchState {
    routes: Vec<(String, search::SearchRoute)>,
    health: Vec<String>,
}

impl SearchState {
    pub fn configure(&mut self, modules: &[CatalogModule], fault: Option<&str>) {
        self.routes.clear();
        self.health.clear();
        for module in modules {
            if !module.nonce_verified && fault != Some("select_search_nonce") {
                continue;
            }
            let Some(raw) = module.role_declarations.get(search::ROLE_DECLARATIONS_KEY) else {
                continue;
            };
            let routes = match search::parse_search_declarations(raw) {
                Ok(slot) => slot.routes,
                Err(error) => {
                    self.health.push(format!("{}: {error}", module.principal));
                    if fault == Some("select_search_declarations") {
                        raw["routes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|value| serde_json::from_value(value.clone()).ok())
                            .collect()
                    } else {
                        continue;
                    }
                }
            };
            self.routes.extend(
                routes
                    .into_iter()
                    .map(|route| (module.principal.clone(), route)),
            );
        }
    }
}

impl FakeRoute {
    pub(super) fn select_search_route(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        let mut params = params.clone();
        if self.fault("select_search_requirements") {
            params["requirements"] = json!({});
        }
        if self.fault("select_search_privacy") {
            params["privacy"] = json!("remote_allowed");
            params.as_object_mut().unwrap().remove("caller_local");
        }
        let allowance = if self.fault("select_search_allowance") {
            params.as_object_mut().unwrap().remove("allowance_left")
        } else {
            None
        };
        let groups: Vec<_> = capabilities::KNOWN
            .iter()
            .copied()
            .filter(|group| self.search_control || *group != capabilities::SELECT_SEARCH)
            .collect();
        let mut request = search::parse_select_search_request(&params, &groups, &["search"])?;
        if let Some(allowance) = allowance {
            request.allowance_left = serde_json::from_value(allowance).ok();
        }
        if !state.consumers.iter().any(|c| c.id == request.consumer)
            && !state.declarations.contains_key(&request.consumer)
        {
            return Err(Refusal::unknown_consumer(&request.consumer));
        }
        if self.principal() != fixture::CONSUMER {
            return Err(Refusal::consumer_not_owned(&request.consumer));
        }
        let default = SearchRouteFacts::new();
        let facts = |route: &search::SearchRoute| {
            state
                .selection_facts
                .search
                .get(&format!("{}/{}", route.provider, route.model))
                .unwrap_or(&default)
        };
        let defaults = state
            .declarations
            .get(&request.consumer)
            .map(|(_, defaults)| defaults);
        let pin = request.pin.as_ref().or_else(|| {
            self.stamp.project_id.as_ref().and_then(|project| {
                state
                    .project_pins
                    .get(&(project.clone(), request.consumer.clone()))
            })
        });
        let level = request
            .requirements
            .iq
            .or_else(|| defaults.and_then(|d| d.iq));
        let mut candidates: Vec<_> = state
            .search
            .routes
            .iter()
            .filter(|(_, route)| {
                let key = format!("{}/{}", route.provider, route.model);
                request.privacy == engine::Privacy::RemoteAllowed
                    && !state
                        .cooldowns
                        .get(&key)
                        .is_some_and(|until| *until > now_ms())
                    && request
                        .exclude
                        .as_ref()
                        .is_none_or(|keys| !keys.contains(&key))
                    && request
                        .exclude_providers
                        .as_ref()
                        .is_none_or(|providers| !providers.contains(&route.provider))
                    && request.eligible_routes.as_ref().is_none_or(|routes| {
                        routes
                            .iter()
                            .any(|r| r.provider == route.provider && r.model == route.model)
                    })
                    && !facts(route).not_for.contains(&request.consumer)
                    && pin.is_none_or(|pin| *pin == key)
            })
            .collect();
        let highest = candidates
            .iter()
            .filter_map(|(_, route)| facts(route).grade)
            .max();
        if pin.is_none() {
            if let Some(level) = level {
                candidates.retain(|(_, route)| {
                    facts(route)
                        .grade
                        .zip(highest)
                        .is_none_or(|(grade, highest)| {
                            u128::from(grade) * 100 >= u128::from(highest) * u128::from(level)
                        })
                });
            }
        }
        let ladder = state
            .ladders
            .get(&request.consumer)
            .or_else(|| defaults.and_then(|d| d.ladder.as_ref()));
        let cost = |route: &search::SearchRoute| {
            if !self.fault("select_search_ranking")
                && request
                    .allowance_left
                    .as_ref()
                    .is_some_and(|routes| routes.contains(route))
            {
                return (false, 0.0, Some(0.0));
            }
            match facts(route)
                .price
                .as_ref()
                .and_then(|price| price.rate(fixture::SEARCH_PRICE_KEY))
            {
                Some(rate) => (
                    false,
                    rate.rate.as_f64().unwrap(),
                    Some(rate.minimum.as_ref().map_or(0.0, |v| v.as_f64().unwrap())),
                ),
                None => (true, 0.0, None),
            }
        };
        candidates.sort_by(|(am, a), (bm, b)| {
            let ladder_pos = |route: &search::SearchRoute| {
                ladder
                    .and_then(|entries| {
                        entries
                            .iter()
                            .position(|key| *key == format!("{}/{}", route.provider, route.model))
                    })
                    .unwrap_or(usize::MAX)
            };
            let cost_cmp = || {
                let ac = cost(a);
                let bc = cost(b);
                ac.0.cmp(&bc.0)
                    .then_with(|| ac.1.total_cmp(&bc.1))
                    .then_with(|| ac.2.partial_cmp(&bc.2).unwrap())
            };
            ladder_pos(a)
                .cmp(&ladder_pos(b))
                .then_with(|| {
                    if level.is_some() {
                        facts(a)
                            .grade
                            .is_none()
                            .cmp(&facts(b).grade.is_none())
                            .then_with(cost_cmp)
                    } else {
                        facts(b).grade.cmp(&facts(a).grade)
                    }
                })
                .then_with(|| facts(b).grade.cmp(&facts(a).grade))
                .then_with(cost_cmp)
                .then_with(|| (am, &a.provider, &a.model).cmp(&(bm, &b.provider, &b.model)))
        });
        let (module, route) = candidates.first().ok_or_else(|| {
            if let Some(pin) = pin {
                Refusal::pin_unavailable(pin, PinUnavailableReason::NotAvailable, None)
            } else {
                Refusal::no_route(NoRouteReason::AllInadequate, None)
            }
        })?;
        let price = facts(route).price.clone();
        let mut selected = engine::SelectedEngine::new(
            module.clone(),
            &engine::Engine::new(
                engine::Kind::Unknown("search".into()),
                route.provider.clone(),
                route.model.clone(),
                false,
                vec![],
                engine::LatencyClass::Interactive,
            ),
        );
        selected.local = false;
        let mut answer = media::SelectMediaAnswer::new(
            state.next("search-decision"),
            now_ms() + engine::DECISION_TTL_MS,
            selected,
        );
        answer.price = price;
        if let Some(price) = &mut answer.price {
            price.estimated_task_usd = None;
        }
        if self.fault("placement_search_wrong_kind")
            && request.consumer == "example-consumer.search"
        {
            answer.engine.model = "model-c".into();
        }
        state.engine.decision_kinds.insert(
            answer.decision_id.clone(),
            engine::Kind::Unknown("search".into()),
        );
        state.engine.decisions.insert(
            answer.decision_id.clone(),
            (
                self.principal().into(),
                engine::SelectEngineAnswer::new(
                    &answer.decision_id,
                    answer.expires_at_ms,
                    answer.engine.clone(),
                ),
            ),
        );
        to_value(answer)
    }
}
