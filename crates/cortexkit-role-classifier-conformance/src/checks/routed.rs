use super::*;
use crate::subject::{RoutedDecision, RoutingObservations, RoutingOutcome};

impl<S: ClassifierSubject> Suite<'_, S> {
    pub(super) async fn routed_check(&self, name: &str) -> Check {
        if !self.describe.declares_routed_models() {
            return Ok(Ending::NotApplicable(
                "provider does not declare routed_models".into(),
            ));
        }
        let model = self.main_model()?.model.clone();
        let first = RoutedDecision {
            model: model.clone(),
            account: "account-1".into(),
            decision_id: self.mint("decision-1"),
        };
        let second = RoutedDecision {
            model: self
                .describe
                .models
                .iter()
                .find(|entry| entry.model != model)
                .map_or(model.clone(), |entry| entry.model.clone()),
            account: "account-2".into(),
            decision_id: self.mint("decision-2"),
        };
        let outcomes = if name == "routed_pin_never_substituted" {
            vec![
                RoutingOutcome::Refused {
                    code: "pin_unavailable".into(),
                    message: "no account for pin".into(),
                },
                RoutingOutcome::Selected(second.clone()),
            ]
        } else if name == "routed_admission_refusals" {
            vec![
                RoutingOutcome::Refused {
                    code: "no_adequate_model".into(),
                    message: "no model meets the requirements".into(),
                },
                RoutingOutcome::Unavailable,
            ]
        } else {
            vec![
                RoutingOutcome::Selected(first.clone()),
                RoutingOutcome::Selected(second),
            ]
        };
        if !self.subject.script_routing(outcomes).await {
            return Ok(Ending::NotApplicable(
                "subject has no routing script hook".into(),
            ));
        }
        let result = self.routed_scenario(name, &first).await;
        self.subject.finish_routing().await;
        result
    }

    async fn routing_log(&self) -> Result<RoutingObservations, String> {
        self.subject
            .routing_observations()
            .await
            .ok_or_else(|| "routing hook supplies no observations".into())
    }

    fn routed_reply(reply: Reply, expected: &RoutedDecision) -> Result<ClassifyReply, String> {
        let (_, reply) = Self::reply_of("routed batch", reply)?;
        if reply.model != expected.model {
            return Err(format!("model {} replaced {}", reply.model, expected.model));
        }
        Ok(reply)
    }

    fn routing_refusal(
        reply: Reply,
        code: &str,
        routing_code: Option<&str>,
        reason: Option<&str>,
        model: Option<&str>,
    ) -> Result<(), String> {
        let Reply::Error(body) = reply else {
            return Err(format!(
                "expected {code} refusal, got a substituted model or reply"
            ));
        };
        if body.code != code {
            return Err(format!("expected {code}, got {}", body.code));
        }
        let detail = body
            .detail
            .as_ref()
            .ok_or("routing refusal missing detail")?;
        for (key, expected) in [
            ("routing_code", routing_code),
            ("routing_reason", reason),
            ("model", model),
        ] {
            if detail.get(key).and_then(Value::as_str) != expected {
                return Err(format!("routing refusal changed {key}"));
            }
        }
        let class = if code == errors::ROUTING_UNAVAILABLE {
            "transient"
        } else {
            "permanent"
        };
        if detail["class"] != class {
            return Err(format!("{code} has wrong class"));
        }
        Ok(())
    }

    async fn routed_scenario(&self, name: &str, first: &RoutedDecision) -> Check {
        let (state, key) = self.marker("routed-item");
        self.script_answer(&key, noul_body("0.75", None)).await;
        let mut request = json!({"batch_id": self.mint("routed"), "questions": noul_question(), "items": items(&[&state])});
        if name == "routed_pin_never_substituted" {
            request["model"] = json!(first.model);
            Self::routing_refusal(
                self.send(&request.to_string()).await?,
                errors::ROUTING_REFUSED,
                Some("pin_unavailable"),
                Some("no account for pin"),
                Some(&first.model),
            )?;
            let log = self.routing_log().await?;
            if log.selections != 1
                || !log.provider_requests.is_empty()
                || self.calls(&key).await != 0
            {
                return Err("pin refusal selected or dispatched a substitute".into());
            }
        } else if name == "routed_admission_refusals" {
            Self::routing_refusal(
                self.send(&request.to_string()).await?,
                errors::ROUTING_REFUSED,
                Some("no_adequate_model"),
                Some("no model meets the requirements"),
                None,
            )?;
            request["batch_id"] = json!(self.mint("routing-down"));
            Self::routing_refusal(
                self.send(&request.to_string()).await?,
                errors::ROUTING_UNAVAILABLE,
                None,
                None,
                None,
            )?;
            if !self.routing_log().await?.provider_requests.is_empty()
                || self.calls(&key).await != 0
            {
                return Err("routing refusal dispatched a provider request".into());
            }
        } else if name == "routed_initial_account_failure_bounded" {
            self.subject.script(&key, Scripted::status(401)).await;
            let (unsent, unsent_key) = self.marker("routed-initial-unsent");
            self.script_answer(&unsent_key, noul_body("0.25", None))
                .await;
            request["items"] = json!(items(&[&state, &unsent]));
            match self.send(&request.to_string()).await? {
                Reply::Error(body) if body.code == errors::AUTH_FAILED => {}
                Reply::Response(text) => {
                    let reply: ClassifyReply =
                        serde_json::from_str(&text).map_err(|e| e.to_string())?;
                    if reply.items.len() != 2
                        || reply.items.iter().any(|item| {
                            item.answers.is_some()
                                || item
                                    .error
                                    .as_ref()
                                    .is_none_or(|e| e.code != errors::AUTH_FAILED)
                        })
                    {
                        return Err(
                            "initial account failure returned an answer or lost the account error"
                                .into(),
                        );
                    }
                }
                _ => return Err("initial account failure lost auth_failed".into()),
            }
            let log = self.routing_log().await?;
            if !(1..=2).contains(&log.selections)
                || !(log.selections..=2 * log.selections).contains(&log.provider_requests.len())
                || self.calls(&key).await != log.provider_requests.len()
                || self.calls(&unsent_key).await != 0
            {
                return Err(
                    "initial account failure exceeded one reselection or one authentication retry per selection, or sent a later item".into(),
                );
            }
        } else if name == "routed_partial_failure_keeps_model" {
            let (failed, failed_key) = self.marker("routed-auth");
            let (unsent, unsent_key) = self.marker("routed-unsent");
            self.subject
                .script(&failed_key, Scripted::status(401))
                .await;
            self.script_answer(&unsent_key, noul_body("0.25", None))
                .await;
            request["items"] = json!(items(&[&state, &failed, &unsent]));
            let reply = Self::routed_reply(self.send(&request.to_string()).await?, first)?;
            if reply.items.len() != 3
                || reply.items[0].answers.is_none()
                || reply.items[1..].iter().any(|item| {
                    item.answers.is_some()
                        || item
                            .error
                            .as_ref()
                            .is_none_or(|e| e.code != errors::AUTH_FAILED)
                })
            {
                return Err("partial account failure did not fail the unanswered items".into());
            }
            let log = self.routing_log().await?;
            let answered_calls = self.calls(&key).await;
            let refused_calls = self.calls(&failed_key).await;
            let unsent_calls = self.calls(&unsent_key).await;
            if log.selections != 1
                || answered_calls != 1
                || !(1..=2).contains(&refused_calls)
                || log.provider_requests.len() != answered_calls + refused_calls
                || log
                    .provider_requests
                    .iter()
                    .any(|decision| decision != first)
                || unsent_calls != 0
            {
                return Err(
                    "partial account failure selected again, mixed decisions or exceeded one authentication retry".into(),
                );
            }
            self.script_answer(&failed_key, noul_body("0.25", None))
                .await;
            let replay = Self::routed_reply(self.send(&request.to_string()).await?, first)?;
            let replay_log = self.routing_log().await?;
            if replay != reply
                || replay_log.selections != log.selections
                || replay_log.provider_requests != log.provider_requests
                || self.calls(&key).await != answered_calls
                || self.calls(&failed_key).await != refused_calls
                || self.calls(&unsent_key).await != unsent_calls
            {
                return Err(
                    "partial account failure was retried instead of replayed from storage".into(),
                );
            }
        } else {
            let reply = Self::routed_reply(self.send(&request.to_string()).await?, first)?;
            if reply.items.len() != 1 || reply.items[0].answers.is_none() {
                return Err("routed item was not answered".into());
            }
            if name == "routed_replay_keeps_decision" {
                let replay = Self::routed_reply(self.send(&request.to_string()).await?, first)?;
                if replay != reply
                    || self.calls(&key).await != 1
                    || self.routing_log().await?.selections != 1
                {
                    return Err("replay selected or dispatched again".into());
                }
            } else {
                if reply.decision_id.as_deref() != Some(&first.decision_id) {
                    return Err("decision_id differs from routing's decision".into());
                }
                // A pin still makes an account decision, so its id is required too.
                request["batch_id"] = json!(self.mint("routed-pinned"));
                request["model"] = json!(first.model);
                if !self
                    .subject
                    .script_routing(vec![RoutingOutcome::Selected(first.clone())])
                    .await
                {
                    return Err("routing hook disappeared".into());
                }
                let pinned = Self::routed_reply(self.send(&request.to_string()).await?, first)?;
                if pinned.decision_id.as_deref() != Some(&first.decision_id) {
                    return Err("pinned account reply is missing decision_id".into());
                }
            }
        }
        Ok(Ending::Passed)
    }
}
