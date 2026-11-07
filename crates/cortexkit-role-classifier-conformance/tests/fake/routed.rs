use super::*;
use cortexkit_role_classifier_conformance::wire::{
    errors::RoutingRefusal,
    reply::{ClassifyReply, ItemResult},
};

#[derive(Default)]
pub(super) struct RoutingState {
    pub active: bool,
    pub outcomes: Vec<RoutingOutcome>,
    pub observations: RoutingObservations,
    pub admission_refusal: Option<RoutingRefusalFixture>,
    pub renewal_refusal: Option<RoutingRefusalFixture>,
    batches: BTreeMap<String, (RoutedDecision, Option<String>)>,
}

impl RoutingState {
    fn select(&mut self) -> RoutingOutcome {
        let position = self.observations.selections;
        self.observations.selections += 1;
        self.outcomes
            .get(position)
            .or_else(|| self.outcomes.last())
            .cloned()
            .unwrap_or(RoutingOutcome::Unavailable)
    }
}

fn routing_error(refusal: RoutingRefusal) -> Reply {
    Reply::Error(
        ErrorBody::new(refusal.code, refusal.message)
            .with_detail(serde_json::to_value(refusal.detail).unwrap()),
    )
}

impl FakeModule {
    fn fixture_refusal(
        &self,
        fixture: RoutingRefusalFixture,
        model: Option<String>,
        renewal: bool,
    ) -> RoutingRefusal {
        let timed_no_route_as_permanent =
            !renewal && self.defects.timed_no_route_as_permanent && fixture.code == "no_route";
        let timed_pool_exhausted_as_permanent = renewal
            && self.defects.timed_pool_exhausted_as_permanent
            && fixture.code == "account_pool_exhausted";
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let mut refusal = RoutingRefusal::refused_with_detail(
            fixture.code,
            Some(fixture.message),
            model,
            &fixture.detail,
            now_ms,
        );
        if timed_no_route_as_permanent || timed_pool_exhausted_as_permanent {
            refusal.code = errors::ROUTING_REFUSED.into();
            refusal.detail.class = errors::ErrorClass::Permanent;
            refusal.detail.retry_after_ms = None;
        }
        if !renewal
            && self.defects.transient_routing_as_permanent
            && refusal.code == errors::ROUTING_DEFERRED
        {
            refusal.code = errors::ROUTING_REFUSED.into();
            refusal.detail.class = errors::ErrorClass::Permanent;
        }
        if !renewal
            && self.defects.permanent_routing_as_transient
            && refusal.code == errors::ROUTING_REFUSED
        {
            refusal.code = errors::ROUTING_DEFERRED.into();
            refusal.detail.class = errors::ErrorClass::Transient;
        }
        if !renewal && self.defects.routing_delay_missing {
            refusal.detail.retry_after_ms = None;
        }
        refusal
    }

    pub(super) async fn classify_routed(&self, text: &str) -> Reply {
        let request = match parse_request(text) {
            Ok(request) => request,
            Err(e) => return refused(e),
        };
        let selected = {
            let mut routing = self.routing.lock().unwrap();
            if let Some(fixture) = routing.admission_refusal.clone() {
                routing.observations.selections += 1;
                return routing_error(self.fixture_refusal(fixture, request.model, false));
            }
            if let Some((decision, Some(reply))) = routing.batches.get(&request.batch_id).cloned() {
                if self.defects.replay_selects_again {
                    routing.select();
                }
                let _ = decision;
                return Reply::Response(reply);
            }
            routing.select()
        };
        let decision = match selected {
            RoutingOutcome::Selected(decision) => decision,
            RoutingOutcome::Unavailable => {
                return routing_error(RoutingRefusal::unavailable("routing timed out"))
            }
            RoutingOutcome::Refused { code, message } => {
                if self.defects.pin_substituted && request.model.is_some() {
                    match self.routing.lock().unwrap().select() {
                        RoutingOutcome::Selected(decision) => decision,
                        _ => panic!("substitution defect needs a fallback choice"),
                    }
                } else {
                    if self.defects.routing_refusal_as_unavailable && request.model.is_none() {
                        return routing_error(RoutingRefusal::unavailable(message));
                    }
                    return routing_error(RoutingRefusal::refused(
                        code,
                        Some(message),
                        request.model,
                    ));
                }
            }
        };
        self.routing
            .lock()
            .unwrap()
            .batches
            .insert(request.batch_id.clone(), (decision.clone(), None));
        let mut used = decision.clone();
        let mut items = Vec::new();
        let mut stopped = false;
        for (index, item) in request.items.iter().enumerate() {
            let renewal = self.routing.lock().unwrap().renewal_refusal.clone();
            if let Some(fixture) = renewal.filter(|_| index > 0) {
                let mut error = self
                    .fixture_refusal(fixture, request.model.clone(), true)
                    .into_item_error()
                    .unwrap();
                if self.defects.renewal_as_provider_error {
                    error = ItemError::for_code(errors::PROVIDER_ERROR, "routing renewal refused");
                }
                if self.defects.renewal_loses_routing_code {
                    error.detail = None;
                }
                if self.defects.renewal_wrong_class {
                    error.class = match error.class {
                        errors::ErrorClass::Transient => errors::ErrorClass::Permanent,
                        _ => errors::ErrorClass::Transient,
                    };
                }
                items.push(ItemResult::failed(index as u64, error));
                continue;
            }
            if stopped {
                items.push(ItemResult::failed(
                    index as u64,
                    ItemError::for_code(errors::AUTH_FAILED, "credential refused"),
                ));
                continue;
            }
            self.routing
                .lock()
                .unwrap()
                .observations
                .provider_requests
                .push(used.clone());
            match self.stand_in.call(&item.state).await {
                Scripted::Answer(body) => {
                    let answer: ProviderAnswer = serde_json::from_str(&body).unwrap();
                    let answers = serde_json::from_str(answer.answers.get()).unwrap();
                    items.push(ItemResult::answered(index as u64, answers));
                }
                Scripted::Status { status, .. } => {
                    if status == 401 {
                        for _ in 0..self.auth_refusal_retries {
                            self.routing
                                .lock()
                                .unwrap()
                                .observations
                                .provider_requests
                                .push(used.clone());
                            self.stand_in.call(&item.state).await;
                        }
                    }
                    if index == 0 && self.defects.initial_account_failure_reselects_twice {
                        for _ in 0..2 {
                            let next = self.routing.lock().unwrap().select();
                            if let RoutingOutcome::Selected(next) = next {
                                self.routing
                                    .lock()
                                    .unwrap()
                                    .observations
                                    .provider_requests
                                    .push(next);
                                self.stand_in.call(&item.state).await;
                            }
                        }
                    }
                    items.push(ItemResult::failed(
                        index as u64,
                        ItemError::for_code(errors::AUTH_FAILED, "credential refused"),
                    ));
                    if self.defects.partial_failure_switches_model && index > 0 {
                        if let RoutingOutcome::Selected(next) =
                            self.routing.lock().unwrap().select()
                        {
                            used = next;
                        }
                    } else {
                        stopped = true;
                    }
                }
            }
        }
        let mut reply = ClassifyReply::new(&request.batch_id, &decision.model, items);
        if !self.defects.missing_decision_id {
            reply = reply.with_decision_id(&decision.decision_id);
        }
        let text = serde_json::to_string(&reply).unwrap();
        self.routing
            .lock()
            .unwrap()
            .batches
            .insert(request.batch_id, (decision, Some(text.clone())));
        Reply::Response(text)
    }
}
