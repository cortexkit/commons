use super::*;
use cortexkit_role_routing_conformance::wire::{
    capabilities, engine_measurements as measurements, select_engine as engine,
    select_media as media,
};

impl FakeRoute {
    pub(super) fn select_media_route(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        let mut params = params.clone();
        if self.fault("select_media_refuses_llm") {
            params["requirements"] = json!({});
        }
        if self.fault("select_media_metric_limit") {
            params["requirements"] = json!({});
        }
        if self.fault("select_media_unlisted_kind") {
            params["kind"] = json!("transcribe");
        }
        if self.fault("select_media_video") {
            params["requirements"] = json!({});
        }
        let mut request = media::parse_select_media_request(
            &params,
            capabilities::KNOWN,
            &["transcribe", "video", "synthesize"],
        )?;
        if self.fault("select_media_privacy") {
            request.privacy = engine::Privacy::RemoteAllowed;
        }
        if !state.consumers.iter().any(|c| c.id == request.consumer)
            && !state.declarations.contains_key(&request.consumer)
        {
            return Err(Refusal::unknown_consumer(&request.consumer));
        }
        if self.principal() != cortexkit_role_routing_conformance::fixture::CONSUMER {
            return Err(Refusal::consumer_not_owned(&request.consumer));
        }
        if self.fault("select_media_requirements") {
            request.requirements.engine = engine::EngineRequirements::new();
        }
        let mut candidates: Vec<engine::EngineCandidate<measurements::EvalScore, u64>> = state
            .engine
            .modules
            .iter()
            .map(|(module, engine)| engine::EngineCandidate::new(module, engine.clone()))
            .collect();
        candidates.extend(
            request
                .caller_local
                .iter()
                .flatten()
                .enumerate()
                .map(|(index, engine)| {
                    engine::EngineCandidate::caller_local(index, engine.clone())
                }),
        );
        let key = |engine: &engine::Engine| format!("{}/{}", engine.provider, engine.model);
        let default_facts = cortexkit_role_routing_conformance::MediaRouteFacts::new();
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
        // A module model pin cannot be redirected to an in-process copy.
        let module_pin = pin.is_some_and(|pin| {
            state
                .engine
                .modules
                .iter()
                .any(|(_, e)| e.kind == request.kind && key(e) == *pin)
        });
        candidates.retain(|candidate| {
            let e = &candidate.engine;
            let route = key(e);
            let facts = state
                .selection_facts
                .media
                .get(&route)
                .unwrap_or(&default_facts);
            e.kind == request.kind
                && engine::meets_requirements(e, &request.requirements.engine)
                && (request.privacy == engine::Privacy::RemoteAllowed || e.local)
                && (e.local
                    || !state
                        .cooldowns
                        .get(&route)
                        .is_some_and(|until| *until > now_ms()))
                && request
                    .requirements
                    .output_duration_ms
                    .is_none_or(|n| facts.max_output_duration_ms.is_some_and(|max| n <= max))
                && request
                    .requirements
                    .aspect_ratio
                    .as_ref()
                    .is_none_or(|s| facts.aspect_ratios.contains(s))
                && request
                    .requirements
                    .audio
                    .is_none_or(|audio| facts.audio == Some(audio))
                && (self.fault("select_media_policy")
                    || (request
                        .exclude
                        .as_ref()
                        .is_none_or(|keys| !keys.contains(&route))
                        && request
                            .exclude_providers
                            .as_ref()
                            .is_none_or(|keys| !keys.contains(&e.provider))
                        && request
                            .eligible_routes
                            .as_ref()
                            .is_none_or(|routes| routes.iter().any(|r| r.route_key() == route))
                        && !facts.not_for.contains(&request.consumer)))
                && (self.fault("select_media_pin")
                    || pin.is_none_or(|pin| {
                        route == *pin && (!module_pin || !candidate.selected.caller_local)
                    }))
        });
        let mut engine_request = request.engine_request();
        if let Some(limit) = &request.requirements.metric_limit {
            engine_request.requirements.metric = Some(limit.metric.clone());
            measurements::apply_evaluations(
                &engine_request,
                &mut candidates,
                &state.engine.evaluations,
            );
            candidates.retain(|c| limit.admits(c.accuracy.map(|s| s.get())));
        }
        engine_request.requirements.metric = request.requirements.engine.metric.clone();
        measurements::apply_evaluations(
            &engine_request,
            &mut candidates,
            &state.engine.evaluations,
        );
        if self.fault("select_media_ranking") {
            for candidate in &mut candidates {
                candidate.accuracy = None;
            }
        }
        for candidate in &mut candidates {
            candidate.cost = state
                .engine
                .facts
                .iter()
                .find(|f| {
                    f.kind == request.kind
                        && f.provider == candidate.engine.provider
                        && f.model == candidate.engine.model
                })
                .and_then(|f| f.cost);
        }
        let highest = candidates
            .iter()
            .filter_map(|c| state.selection_facts.media.get(&key(&c.engine))?.grade)
            .max();
        let level = request
            .requirements
            .iq
            .or_else(|| defaults.and_then(|d| d.iq));
        if pin.is_none() && !self.fault("select_media_demand") {
            if let Some(level) = level {
                candidates.retain(|c| {
                    let grade = state
                        .selection_facts
                        .media
                        .get(&key(&c.engine))
                        .and_then(|f| f.grade);
                    if self.fault("select_media_ungraded_level") && grade.is_none() {
                        return false;
                    }
                    self.fault("select_media_ungraded_below_level")
                        || grade.zip(highest).is_none_or(|(grade, highest)| {
                            (grade as u128) * 100 >= (highest as u128) * u128::from(level)
                        })
                });
            }
        }
        let ladder = if self.fault("select_media_ladder") {
            None
        } else {
            state
                .ladders
                .get(&request.consumer)
                .or_else(|| defaults.and_then(|d| d.ladder.as_ref()))
        };
        candidates.sort_by(|a, b| {
            let facts = |c: &engine::EngineCandidate<measurements::EvalScore, u64>| {
                state
                    .selection_facts
                    .media
                    .get(&key(&c.engine))
                    .and_then(|f| f.grade)
            };
            let ladder_pos = |c: &engine::EngineCandidate<measurements::EvalScore, u64>| {
                ladder
                    .and_then(|entries| entries.iter().position(|entry| entry == &key(&c.engine)))
                    .unwrap_or(usize::MAX)
            };
            let cost = |c: &engine::EngineCandidate<measurements::EvalScore, u64>| {
                (c.cost.is_none(), c.cost)
            };
            ladder_pos(a)
                .cmp(&ladder_pos(b))
                .then_with(|| {
                    if level.is_some() && !self.fault("select_media_demand") {
                        let unknown = |candidate| {
                            !self.fault("select_media_ungraded_cost_first")
                                && facts(candidate).is_none()
                        };
                        unknown(a)
                            .cmp(&unknown(b))
                            .then_with(|| cost(a).cmp(&cost(b)))
                    } else {
                        facts(b).cmp(&facts(a))
                    }
                })
                .then_with(|| facts(b).cmp(&facts(a)))
                .then_with(|| b.accuracy.cmp(&a.accuracy))
                .then_with(|| cost(a).cmp(&cost(b)))
                .then_with(|| {
                    (
                        &a.selected.module,
                        &a.engine.provider,
                        &a.engine.model,
                        a.selected.index,
                    )
                        .cmp(&(
                            &b.selected.module,
                            &b.engine.provider,
                            &b.engine.model,
                            b.selected.index,
                        ))
                })
        });
        let candidate = candidates.first().ok_or_else(|| {
            if let Some(pin) = pin {
                Refusal::pin_unavailable(pin, PinUnavailableReason::NotAvailable, None)
            } else {
                Refusal::no_route(NoRouteReason::AllInadequate, None)
            }
        })?;
        let selected = candidate.selected.clone();
        let mut answer = media::SelectMediaAnswer::new(
            state.next("media-decision"),
            now_ms() + engine::DECISION_TTL_MS,
            selected,
        );
        answer.price = state
            .selection_facts
            .media
            .get(&key(&candidate.engine))
            .and_then(|f| f.price.clone());
        // A real router computes rates as f64, so an integral rate goes out as
        // `2.0`, not `2`. Emit rates the same way so a check that compares the
        // integer spelling fails here first.
        if let Some(price) = &mut answer.price {
            for unit in price.rates.values_mut() {
                unit.rate = serde_json::Number::from_f64(unit.rate.as_f64().unwrap()).unwrap();
            }
        }
        if request.kind == engine::Kind::Synthesize {
            if let Some(price) = &mut answer.price {
                let count = if self.fault("select_media_input_characters") {
                    request.requirements.engine.input_bytes
                } else {
                    request
                        .requirements
                        .input_characters
                        .or(request.requirements.engine.input_bytes)
                };
                if let Some((count, rate)) = count.zip(price.rate("input.per_character")) {
                    price.estimated_task_usd =
                        serde_json::Number::from_f64(count as f64 * rate.rate.as_f64().unwrap());
                }
            }
        }
        if self.fault("select_media_price") {
            answer.price = None;
        }
        if self.fault("select_media_answer") {
            answer.expires_at_ms += 60_000;
        }
        if !self.fault("select_media_report_admission") {
            state
                .engine
                .decision_kinds
                .insert(answer.decision_id.clone(), request.kind);
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
        }
        to_value(answer)
    }
}
