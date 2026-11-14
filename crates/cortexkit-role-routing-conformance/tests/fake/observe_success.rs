use super::*;
use cortexkit_role_routing_conformance::wire::{
    attempt::AttemptStatus, observe::parse_account_observe_request_serving,
};

impl FakeRoute {
    pub(super) fn observe_success(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        if !self.is_runner(state) {
            return Err(Refusal::new(
                RefusalCode::AccountNotAuthorized,
                "runner required",
            ));
        }
        let bypass = [
            "account_observe_success_undeclared",
            "account_observe_success_status_refusals",
            "account_observe_success_companions",
            "account_observe_success_kind",
            "account_observe_success_retry",
        ]
        .iter()
        .any(|name| self.fault(name));
        if bypass {
            return to_value(AccountObserveAnswer {
                observation: Observation {
                    id: state.next("wrong-success"),
                    duplicate: false,
                    bound: false,
                    effects: vec![],
                },
            });
        }
        let request = parse_account_observe_request_serving(
            params,
            if self.extensions {
                extensions::GROUPS
            } else {
                &[]
            },
        )?;
        let key = (self.principal().to_owned(), request.event_id.clone());
        let duplicate = state.observations.get(&key);
        if !self.fault("account_observe_success_duplicate") {
            if let Some(first) = duplicate {
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
        let admission = (
            self.principal().to_owned(),
            request.admission_id.clone().unwrap(),
        );
        let pair = (
            request.served_account_id.unwrap(),
            request.credential_id.unwrap(),
        );
        let bound = state.admissions.get(&admission).is_some_and(|(_, answer)| {
            answer.account.account_id == pair.0 && answer.account.credential_id == pair.1
        });
        let status = request.attempt_outcome.unwrap().status;
        let served = matches!(status, AttemptStatus::Served | AttemptStatus::ServedEmpty)
            || self.fault("account_observe_success_unknown");
        let captured = state.observe_retests.get(&admission);
        let pool = state
            .admissions
            .get(&admission)
            .and_then(|(request, _)| request.pool.clone());
        let current = state
            .observe_block(&pair, &pool)
            .map(|(scope, block)| (scope.clone(), block.clone()));
        let matching = captured.is_some_and(|block| Some(block) == current.as_ref());
        let wrong_scope = self.fault("account_observe_success_newer_block")
            || self.fault("account_observe_success_unbound")
            || self.fault("account_observe_success_duplicate");
        let mut effects = vec![];
        if served
            && (bound || self.fault("account_observe_success_unbound"))
            && (matching || wrong_scope)
            && !self.fault("account_observe_success_clears")
        {
            if let Some((scope, (kind, _))) = current {
                state.observe_blocks.remove(&scope);
                effects.push(format!("ended {kind} block"));
                if self.fault("account_observe_success_other_credential") {
                    if kind == "quota" {
                        state.observe_blocks.clear();
                    } else {
                        state
                            .observe_blocks
                            .retain(|scope, _| scope.account() != pair.0);
                    }
                }
                if self.fault("account_observe_success_other_account") {
                    state.observe_blocks.clear();
                }
            }
        }
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
