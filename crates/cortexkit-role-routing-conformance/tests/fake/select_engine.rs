use super::*;
use cortexkit_role_routing_conformance::fixture;
use cortexkit_role_routing_conformance::wire::engine_measurements as measurements;
use cortexkit_role_routing_conformance::{wire::select_engine as engine, EngineFacts};

#[derive(Default)]
pub(super) struct EngineState {
    modules: Vec<(String, engine::Engine)>,
    principals: Vec<String>,
    pub facts: Vec<EngineFacts>,
    pub elapsed: u64,
    decisions: HashMap<String, (String, engine::SelectEngineAnswer)>,
    decision_kinds: HashMap<String, engine::Kind>,
    admissions: HashMap<(String, String), (Value, Value)>,
    evaluations: Vec<measurements::EngineEvalRequest>,
}

impl EngineState {
    pub fn configure(&mut self, modules: &[CatalogModule], fault: &str) {
        self.modules.clear();
        self.principals.clear();
        for module in modules {
            if !module.nonce_verified && fault != "select_engine_slot_unverified" {
                continue;
            }
            let Some(raw) = module.role_declarations.get(engine::ROLE_DECLARATIONS_KEY) else {
                continue;
            };
            let parsed = cortexkit_role_routing_conformance::wire::engine_declarations::parse_engine_declarations(raw);
            let engines = match parsed {
                Ok(slot) => slot.engines,
                Err(_)
                    if matches!(
                        fault,
                        "select_engine_slot_malformed"
                            | "select_engine_slot_duplicate"
                            | "select_engine_slot_empty_formats"
                    ) =>
                {
                    raw["engines"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|entry| serde_json::from_value(entry.clone()).ok())
                        .collect()
                }
                Err(_) => continue,
            };
            self.principals.push(module.principal.clone());
            self.modules.extend(
                engines
                    .into_iter()
                    .map(|engine| (module.principal.clone(), engine)),
            );
        }
    }
}

impl FakeRoute {
    pub(super) fn engine_op(
        &self,
        state: &mut State,
        method: &str,
        params: &Value,
    ) -> Option<Result<Value, Refusal>> {
        let decision = params
            .get("decision_id")
            .and_then(Value::as_str)
            .and_then(|id| state.engine.decisions.get(id))
            .cloned();
        match method {
            ops::ROUTE_SELECT_ENGINE => Some(self.select_media(state, params)),
            ops::ROUTE_ENGINE_EVAL | ops::ROUTE_ENGINE_REPORT => {
                Some(self.measure_engine(state, method, params))
            }
            ops::ROUTE_REPORT if decision.is_some() => {
                let (principal, _) = decision.unwrap();
                Some(
                    if principal != self.principal()
                        && self.defects.engine_fault != "select_engine_report"
                    {
                        Err(Refusal::unknown_decision())
                    } else {
                        parse_report_request(params).map(|request| json!({"decision_id":request.decision_id, "report_id":"engine-report", "outcome":request.outcome}))
                    },
                )
            }
            ops::ROUTE_EXPLAIN if decision.is_some() => {
                Some(if self.defects.engine_fault == "select_engine_explain" {
                    Ok(json!({}))
                } else {
                    Err(Refusal::unknown_decision())
                })
            }
            ops::ROUTE_ACCOUNT_FOR
                if state
                    .engine
                    .principals
                    .iter()
                    .any(|p| p == self.principal())
                    || decision.is_some()
                    || self.principal() == "example-media"
                    || self.principal() == "other-media" =>
            {
                Some(self.media_account(state, params))
            }
            _ => None,
        }
    }

    fn select_media(&self, state: &mut State, params: &Value) -> Result<Value, Refusal> {
        let fault = self.defects.engine_fault;
        let mut params = params.clone();
        if fault == "select_engine_requirements_valid" {
            params["requirements"] = json!({});
        }
        if fault == "select_engine_caller_local" {
            params["caller_local"][0]["local"] = json!(true);
        }
        let mut request = engine::parse_select_engine_request(&params)?;
        if fault != "select_engine_request" {
            if !state.consumers.iter().any(|c| c.id == request.consumer) {
                return Err(Refusal::unknown_consumer(&request.consumer));
            }
            if self.principal() != fixture::CONSUMER {
                return Err(Refusal::consumer_not_owned(&request.consumer));
            }
        }
        if fault == "select_engine_privacy" {
            request.privacy = engine::Privacy::RemoteAllowed;
        }
        if fault == "select_engine_match_requirements" {
            request.requirements = engine::EngineRequirements::new();
        }
        if let Some(member) = fault.strip_prefix("requirement:") {
            let mut requirements = serde_json::to_value(&request.requirements).unwrap();
            requirements.as_object_mut().unwrap().remove(member);
            request.requirements = serde_json::from_value(requirements).unwrap();
        }
        let mut candidates: Vec<engine::EngineCandidate<measurements::EvalScore, u64>> = state
            .engine
            .modules
            .iter()
            .map(|(module, value)| engine::EngineCandidate::new(module, value.clone()))
            .collect();
        candidates.extend(
            request
                .caller_local
                .iter()
                .flatten()
                .enumerate()
                .map(|(index, value)| engine::EngineCandidate::caller_local(index, value.clone())),
        );
        for candidate in &mut candidates {
            if fault != "select_engine_ranking" {
                if let Some(facts) = state.engine.facts.iter().find(|fact| {
                    fact.kind == candidate.engine.kind
                        && fact.provider == candidate.engine.provider
                        && fact.model == candidate.engine.model
                }) {
                    candidate.cost = facts.cost;
                }
            }
            candidate.fenced = fault != "select_engine_reachability"
                && state
                    .cooldowns
                    .get(&format!(
                        "{}/{}",
                        candidate.engine.provider, candidate.engine.model
                    ))
                    .is_some_and(|until| *until > now_ms() + state.engine.elapsed);
        }
        let mut evaluations = state.engine.evaluations.clone();
        match fault {
            "select_engine_eval_measured" => evaluations.clear(),
            "select_engine_eval_higher_score" => {
                for eval in &mut evaluations {
                    eval.score = 1.0 - eval.score;
                }
            }
            "select_engine_eval_different_set" => {
                for eval in &mut evaluations {
                    eval.set.id = "all-sets".into();
                }
            }
            "select_engine_eval_new_model" => {
                for eval in &mut evaluations {
                    if eval.engine.model == "a-old" {
                        eval.engine.model = "a-new".into();
                    }
                }
            }
            "select_engine_eval_default_metric" if request.requirements.metric.is_none() => {
                request.requirements.metric = Some("named_term_recall".into())
            }
            "select_engine_eval_requested_metric" => request.requirements.metric = None,
            "select_engine_eval_newest_set" => {
                evaluations.retain(|eval| eval.set.id == "old");
            }
            _ => {}
        }
        measurements::apply_evaluations(&request, &mut candidates, &evaluations);
        let selected = engine::select_engine(&request, &candidates)
            .map_err(|mut refusal| {
                if fault == "select_engine_filters" {
                    refusal
                        .detail
                        .insert("filter".into(), json!("wrong-filter"));
                }
                refusal
            })?
            .selected
            .clone();
        let id = state.next("engine-decision");
        let answer = engine::SelectEngineAnswer::new(
            &id,
            now_ms()
                + state.engine.elapsed
                + engine::DECISION_TTL_MS
                + if fault == "select_engine_answer" {
                    60_000
                } else {
                    0
                },
            selected,
        );
        state.engine.decision_kinds.insert(id.clone(), request.kind);
        state
            .engine
            .decisions
            .insert(id, (self.principal().into(), answer.clone()));
        to_value(answer)
    }

    fn media_account(&self, state: &mut State, params: &Value) -> Result<Value, Refusal> {
        let fault = self.defects.engine_fault;
        let provider = params["provider"].as_str().unwrap();
        let model = params["model"].as_str().unwrap();
        let authorized = state.engine.modules.iter().any(|(principal, engine)| {
            principal == self.principal()
                && !engine.local
                && engine.provider == provider
                && engine.model == model
        });
        if (!authorized && fault != "select_engine_media_admission")
            || fault == "select_engine_admission_before_expiry"
        {
            return Err(Refusal::new(
                RefusalCode::AccountNotAuthorized,
                "not a remote engine in this attested slot",
            ));
        }
        let key = (
            self.principal().to_owned(),
            params["admission_id"].as_str().unwrap().to_owned(),
        );
        if fault != "select_engine_admission_replay" {
            if let Some((first, answer)) = state.engine.admissions.get(&key) {
                return if first == params {
                    Ok(answer.clone())
                } else {
                    Err(Refusal::new(
                        RefusalCode::AdmissionConflict,
                        "changed admission body",
                    ))
                };
            }
        }
        if let Some(id) = params.get("decision_id").and_then(Value::as_str) {
            let (_, answer) = state
                .engine
                .decisions
                .get(id)
                .ok_or_else(Refusal::unknown_decision)?;
            let selected = &answer.engine;
            if (selected.module.as_deref() != Some(self.principal())
                && fault != "select_engine_admission_foreign"
                && fault != "select_engine_admission_caller_local")
                || (selected.caller_local && fault != "select_engine_admission_caller_local")
                || ((selected.provider != provider || selected.model != model)
                    && fault != "select_engine_admission_pair")
                || (now_ms() + state.engine.elapsed >= answer.expires_at_ms
                    && fault != "select_engine_admission_expiry")
            {
                return Err(Refusal::unknown_decision());
            }
        }
        let chosen = &Self::catalog(state).accounts[0];
        let answer = AccountForAnswer::new(
            Admission {
                id: key.1.clone(),
                state: if fault == "reserved_admission_state" {
                    "reserved"
                } else {
                    "admitted"
                }
                .into(),
            },
            Account {
                provider: chosen.provider.clone(),
                account_id: chosen.account_id.clone(),
                credential_id: chosen.credential_id.clone(),
                method: chosen.method.clone(),
            },
            AccountPin::new(
                "engine-pin",
                1,
                now_ms() + 3_600_000,
                now_ms() + 600_000,
                now_ms() + 300_000,
                AccountMode::Admit,
            ),
            AccountReason {
                code: AccountReasonCode::Decision,
                decision_id: params
                    .get("decision_id")
                    .and_then(Value::as_str)
                    .unwrap_or("allocation")
                    .into(),
                policy_version: "fake-1".into(),
                quota_fetched_at_ms: None,
                quota_age_band: "unknown".into(),
            },
        );
        let value = serde_json::to_value(answer).unwrap();
        state
            .engine
            .admissions
            .insert(key, (params.clone(), value.clone()));
        Ok(value)
    }

    fn measure_engine(
        &self,
        state: &mut State,
        method: &str,
        params: &Value,
    ) -> Result<Value, Refusal> {
        let fault = self.defects.engine_fault;
        if method == ops::ROUTE_ENGINE_EVAL {
            let request = if fault == "select_engine_eval_score_range" {
                let mut body = params.clone();
                body["score"] = json!(0.5);
                measurements::parse_engine_eval_request(&body)?
            } else {
                measurements::parse_engine_eval_request(params)?
            };
            if fault != "select_engine_eval_authorization" {
                measurements::check_engine_authorization(
                    self.principal(),
                    &request.kind,
                    &request.engine,
                    state.engine.modules.iter().map(|(p, e)| (p.as_str(), e)),
                )?;
            }
            state.engine.evaluations.push(request);
        } else {
            let request = measurements::parse_engine_report_request(params)?;
            if fault != "select_engine_speed_authorization" {
                measurements::check_engine_authorization(
                    self.principal(),
                    &request.kind,
                    &request.engine,
                    state.engine.modules.iter().map(|(p, e)| (p.as_str(), e)),
                )?;
            }
            let (_, decision) = state
                .engine
                .decisions
                .get(&request.decision_id)
                .ok_or_else(Refusal::unknown_decision)?;
            if decision.engine.caller_local
                || decision.engine.module.as_deref() != Some(self.principal())
                || state.engine.decision_kinds.get(&request.decision_id) != Some(&request.kind)
                || decision.engine.provider != request.engine.provider
                || decision.engine.model != request.engine.model
            {
                return Err(Refusal::unknown_decision());
            }
        }
        Ok(json!({}))
    }
}
