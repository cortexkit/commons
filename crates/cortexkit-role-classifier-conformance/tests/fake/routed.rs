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
    pub(super) async fn classify_routed(&self, text: &str) -> Reply {
        let request = match parse_request(text) {
            Ok(request) => request,
            Err(e) => return refused(e),
        };
        let selected = {
            let mut routing = self.routing.lock().unwrap();
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
                Scripted::Status { .. } => {
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
