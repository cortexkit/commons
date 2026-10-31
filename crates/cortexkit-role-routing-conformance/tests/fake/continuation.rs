use super::*;
use cortexkit_role_routing_conformance::wire::account_continue::{
    parse_continue_request, ContinueAnswer, ContinueHold, ContinueOffer, ContinueRefusal,
};

impl FakeRoute {
    pub(super) fn account_continue(
        &self,
        state: &mut State,
        params: &Value,
    ) -> Result<Value, Refusal> {
        if !self.is_runner(state) {
            return Err(Refusal::new(
                RefusalCode::AccountNotAuthorized,
                "only runners continue",
            ));
        }
        let mut describe: RoleDescribe = serde_json::from_value(self.describe(state)?).unwrap();
        if self.defects.continue_undeclared {
            describe = describe.with_capability("account_continue");
        }
        let request = parse_continue_request(params, &describe)?;
        if !self.continue_control && self.defects.continue_undeclared {
            return to_value(ContinueAnswer::Hold(ContinueHold::new(
                "account_pool_exhausted",
                json!({}),
            )));
        }
        let key = (self.principal().into(), request.selection_id.clone());
        if let Some((first_request, first_answer)) = state.continue_answers.get(&key) {
            if !first_request.same_question(&request) && !self.defects.continue_changed_question {
                return Err(Refusal::invalid_request(
                    "selection_id",
                    "the selection id was used with a different question",
                ));
            }
            if !self.defects.continue_retry {
                return to_value(first_answer);
            }
        }
        let admission_key = (self.principal().into(), request.admission_id.clone());
        let Some((admission, admitted)) = state.admissions.get(&admission_key) else {
            return to_value(ContinueAnswer::Refuse(ContinueRefusal::new(
                "unknown_decision",
                json!({}),
            )));
        };
        let Some(previous) = state.decisions.get(&request.previous_decision_id) else {
            return to_value(ContinueAnswer::Refuse(ContinueRefusal::new(
                "unknown_decision",
                json!({}),
            )));
        };
        if previous.runner != self.principal()
            || admission.decision_id.as_deref() != Some(&request.previous_decision_id)
            || admission.conversation_key != request.conversation_key
        {
            return to_value(ContinueAnswer::Refuse(ContinueRefusal::new(
                "unknown_decision",
                json!({}),
            )));
        }
        let route = previous.route.clone();
        let principal = previous.principal.clone();
        let hold_cap = previous.hold_cap;
        let method = admitted.account.method.clone();
        state.blocks.insert(
            request.exhausted.account.clone(),
            request.exhausted.resets_at_ms,
        );
        let forced = if self.defects.continue_exhausted {
            Some(request.exhausted.account.as_str())
        } else if self.defects.continue_provider {
            Some("account-3")
        } else if self.defects.continue_auth {
            Some("account-4")
        } else if self.defects.continue_model {
            Some("account-5")
        } else if self.defects.continue_held {
            Some("account-6")
        } else {
            None
        };
        let account = state
            .continue_credentials
            .iter()
            .find(|c| {
                c.account.account_id != request.exhausted.account
                    && c.account.provider == route.provider
                    && c.account.method == method
                    && c.models.contains(&route.model)
                    && !state
                        .blocks
                        .get(&c.account.account_id)
                        .is_some_and(|reset| reset.is_none_or(|r| r > now_ms()))
            })
            .map(|eligible| {
                forced
                    .and_then(|id| {
                        state
                            .continue_credentials
                            .iter()
                            .find(|c| c.account.account_id == id)
                    })
                    .unwrap_or(eligible)
                    .account
                    .clone()
            });
        let answer = if let Some(account) = account {
            let decision_id = state.next("continue-decision");
            state.decisions.insert(
                decision_id.clone(),
                Decision {
                    principal,
                    route,
                    runner: self.principal().into(),
                    hold_cap,
                },
            );
            ContinueAnswer::Offer(ContinueOffer::new(
                account.credential_id,
                account.account_id,
                decision_id,
                now_ms() + 60_000,
            ))
        } else {
            let mut hold = ContinueHold::new("account_pool_exhausted", json!({}));
            if self.defects.continue_unknown_now {
                hold.until_ms = Some(now_ms());
            }
            ContinueAnswer::Hold(hold)
        };
        state
            .continue_answers
            .insert(key, (request, answer.clone()));
        to_value(answer)
    }
}
