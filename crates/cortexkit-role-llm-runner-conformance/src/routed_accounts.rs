//! Scripted routing responses and live checks for runner-owned account admission.
//!
//! Routing bodies remain JSON so adapters do not need a routing contract dependency.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

use cortexkit_role_harness::HarnessError;
use cortexkit_role_llm_runner::send::AccountRoute;
use serde_json::{json, Value};

use crate::{
    cases::{Case, Ending},
    drive::head,
    subject::{LlmRunnerSubject, Script, ScriptedPart, ScriptedTurn},
    RunnerRoute,
};

/// One actual routing request, including the authenticated calling runner.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct RoutingCall {
    pub runner: String,
    pub method: String,
    pub body: Value,
}

#[derive(Debug, Default)]
struct State {
    calls: Vec<RoutingCall>,
    observations: BTreeMap<(String, String), String>,
    admissions: BTreeMap<(String, String), (Value, Value)>,
    pins: BTreeMap<(String, String), Value>,
    account_override: Option<(String, String)>,
    account_refusal: Option<String>,
}

/// A session's scripted routing peer. Clones share recordings and replay keys.
/// Install it with [`LlmRunnerSubject::install_routing_peer`], forwarding real
/// requests with the authenticated runner identity to [`Self::request`].
#[derive(Clone, Debug)]
pub struct RecordingRoutingPeer {
    method: String,
    fixture: Value,
    account_route: Option<AccountRoute>,
    state: Arc<Mutex<State>>,
}

impl RecordingRoutingPeer {
    /// Bind a sign-in method to public ids returned by
    /// [`LlmRunnerSubject::routed_account_fixture`]. No credential secret belongs here.
    pub fn new(method: impl Into<String>, fixture: Value) -> Result<Self, HarnessError> {
        for field in ["provider", "account_id", "credential_id"] {
            required_string(&fixture, field)?;
        }
        Ok(Self {
            method: method.into(),
            fixture,
            account_route: None,
            state: Arc::new(Mutex::new(State::default())),
        })
    }

    /// The sign-in method the session must exercise.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Public account ids; the adapter configures a compatible provider/model.
    pub fn fixture(&self) -> &Value {
        &self.fixture
    }

    /// Use a separate routing namespace while keeping `fixture()` as the
    /// vendor configuration. Replies name this route's provider but keep the
    /// fixture's provisioned account and credential ids.
    pub fn with_account_route(mut self, route: AccountRoute) -> Self {
        self.account_route = Some(route);
        self
    }

    /// The synthetic routing names a send must forward, when this peer is
    /// exercising `account_route`. The model call still uses the vendor fixture.
    pub fn account_route(&self) -> Option<&AccountRoute> {
        self.account_route.as_ref()
    }

    /// A snapshot of every received body, including retries, in arrival order.
    pub fn calls(&self) -> Vec<RoutingCall> {
        self.state.lock().unwrap().calls.clone()
    }

    /// Answer new admissions with a scripted public credential id and tier.
    /// Existing admission-id replays retain their first answer. `None` restores
    /// the fixture credential and the request's tier.
    pub fn set_account_override(&self, account: Option<(String, String)>) {
        self.state.lock().unwrap().account_override = account;
    }

    /// Refuse fresh admissions with this routing code, or restore successful
    /// answers with `None`. Opted-in adapters send the returned `error` member
    /// as an ERROR frame, not a RESPONSE containing an error-shaped object.
    pub fn set_account_refusal(&self, code: Option<String>) {
        self.state.lock().unwrap().account_refusal = code;
    }

    /// Record a real request and return its JSON RESPONSE body (`{result: ...}`).
    /// `runner` comes from the authenticated route, never a body member.
    /// Supports `route.account_for` admit/renew and `route.account_observe`.
    /// This scripted peer is not a general-purpose routing implementation.
    pub fn request(&self, runner: &str, method: &str, body: Value) -> Result<Value, HarnessError> {
        let mut state = self.state.lock().unwrap();
        state.calls.push(RoutingCall {
            runner: runner.into(),
            method: method.into(),
            body: body.clone(),
        });
        match method {
            "route.account_for" => self.account_for(&mut state, runner, body),
            "route.account_observe" => {
                let event = required_string(&body, "event_id")?;
                let key = (runner.to_owned(), event.to_owned());
                let duplicate = state.observations.contains_key(&key);
                let fresh_id = format!("observation-{}", state.observations.len() + 1);
                let id = state.observations.entry(key).or_insert(fresh_id);
                Ok(json!({"result": {"observation": {
                    "id": id, "duplicate": duplicate, "bound": true, "effects": []
                }}}))
            }
            _ => Err(HarnessError::new(format!(
                "unsupported routing method {method}"
            ))),
        }
    }

    fn account_for(
        &self,
        state: &mut State,
        runner: &str,
        body: Value,
    ) -> Result<Value, HarnessError> {
        let admission = required_string(&body, "admission_id")?;
        let conversation = required_string(&body, "conversation_key")?;
        if let Some(route) = &self.account_route {
            if body.get("provider") != Some(&json!(route.provider))
                || body.get("model") != Some(&json!(route.model))
            {
                return Err(HarnessError::new(
                    "route.account_for did not forward account_route provider and model",
                ));
            }
        } else if body.get("provider") != self.fixture.get("provider") {
            return Err(HarnessError::new(
                "account_for provider differs from fixture",
            ));
        }
        required_string(&body, "model")?;
        let mode = body.get("mode").and_then(Value::as_str).unwrap_or("admit");
        if !matches!(mode, "admit" | "renew") {
            return Err(HarnessError::new(
                "recording peer supports only admit and renew",
            ));
        }
        let key = (runner.to_owned(), admission.to_owned());
        let mut normalized = body.clone();
        let members = normalized
            .as_object_mut()
            .ok_or_else(|| HarnessError::new("expected routing object"))?;
        members.retain(|_, value| !value.is_null());
        members.insert("mode".into(), json!(mode));
        if !members.contains_key("service_tier") {
            members.insert("service_tier".into(), json!("standard"));
        }
        if let Some((first, answer)) = state.admissions.get(&key) {
            return if first == &normalized {
                Ok(answer.clone())
            } else {
                Err(HarnessError::new("admission_conflict"))
            };
        }
        if let Some(code) = &state.account_refusal {
            return Ok(json!({"error":{"code":code,"message":"scripted routing refusal"}}));
        }
        let pin_key = (runner.to_owned(), conversation.to_owned());
        let previous = state.pins.get(&pin_key);
        if mode == "renew"
            && !previous.is_some_and(|pin| {
                body.pointer("/expected_pin/id") == pin.get("id")
                    && body.pointer("/expected_pin/generation") == pin.get("generation")
            })
        {
            return Err(HarnessError::new("pin_generation_conflict"));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| HarnessError::new(e.to_string()))?
            .as_millis() as u64;
        let lease = (now + 300_000).max(
            previous
                .and_then(|pin| pin["lease_valid_until_ms"].as_u64())
                .unwrap_or(0)
                + 1,
        );
        let pin = json!({
            "id": previous.map(|pin| pin["id"].clone()).unwrap_or_else(|| json!(format!("pin-{}", state.pins.len() + 1))),
            "generation": previous.and_then(|pin| pin["generation"].as_u64()).unwrap_or(0) + 1,
            "affinity_expires_at_ms": lease + 3_600_000,
            "admission_valid_until_ms": lease + 300_000,
            "lease_valid_until_ms": lease,
            "mode": mode
        });
        let mut answer = json!({"result": {
            "admission": {"id": admission, "state": "admitted"},
            "account": {
                "provider": body["provider"], "account_id": self.fixture["account_id"],
                "credential_id": self.fixture["credential_id"], "method": self.method
            },
            "pin": pin,
            "reason": {
                "code": if mode == "renew" { "renewed" } else { "headroom" },
                "decision_id": body.get("decision_id").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(format!("allocation-{}", state.admissions.len() + 1))),
                "policy_version": "recording-peer-1", "quota_age_band": "unknown"
            }
        }});
        if let Some(tier) = body.get("service_tier").filter(|v| !v.is_null()) {
            answer["result"]["service_tier_chosen"] = tier.clone();
        }
        if let Some((credential, tier)) = &state.account_override {
            answer["result"]["account"]["credential_id"] = json!(credential);
            answer["result"]["service_tier_chosen"] = json!(tier);
        }
        state.pins.insert(pin_key, pin);
        state.admissions.insert(key, (normalized, answer.clone()));
        Ok(answer)
    }
}

fn required_string<'a>(value: &'a Value, field: &str) -> Result<&'a str, HarnessError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| HarnessError::new(format!("routing fixture/request requires {field}")))
}

pub(crate) async fn run<S: LlmRunnerSubject>(
    case: &Case<'_, S>,
    name: &str,
) -> Result<Ending, String>
where
    S::Route: RunnerRoute,
{
    let describe = case.checked_describe().await?;
    let Some(methods) = describe.routed_accounts else {
        return Ok(Ending::Inapplicable(
            "routed_accounts is not declared on the wire".into(),
        ));
    };
    match name {
        "routed_accounts_follows_account_choice" => {
            let declaration = case
                .subject
                .routing_runner_declaration()
                .await
                .map_err(|e| format!("routing_runner_declaration: {e}"))?;
            if declaration.get("follows_account_choice") != Some(&Value::Bool(true)) {
                return Err("routed methods require follows_account_choice: true in the routing runner declaration".into());
            }
        }
        "routed_accounts_account_for" => {
            for method in methods {
                let fixture = case
                    .subject
                    .routed_account_fixture(&method)
                    .await
                    .map_err(|e| format!("routed_account_fixture({method}): {e}"))?;
                let peer = RecordingRoutingPeer::new(&method, fixture)
                    .map_err(|e| format!("routed_account_fixture({method}): {e}"))?;
                let mut session = case
                    .open(
                        "routed-account",
                        Script {
                            turns: vec![ScriptedTurn::new(vec![ScriptedPart::Text(
                                "routed account accepted".into(),
                            )])],
                        },
                    )
                    .await?;
                case.subject
                    .install_routing_peer(&session.name, peer.clone())
                    .await
                    .map_err(|e| format!("install_routing_peer({method}): {e}"))?;
                let (_, end) = case.run_prompt(&mut session, "answer briefly").await?;
                let calls = peer.calls();
                if !calls.iter().any(|call| {
                    call.method == "route.account_for"
                        && call
                            .body
                            .get("mode")
                            .and_then(Value::as_str)
                            .unwrap_or("admit")
                            == "admit"
                        && call.body.get("provider") == peer.fixture().get("provider")
                }) {
                    return Err(format!("listed method {method} sent no route.account_for admission for the fixture provider"));
                }
                if case
                    .subject
                    .model_invocations(&session.name)
                    .await
                    .map_err(|e| e.to_string())?
                    == 0
                {
                    return Err(format!(
                        "listed method {method} never reached the scripted model"
                    ));
                }
                if !head(&session.route)
                    .await?
                    .last_run_state
                    .is_some_and(|last| last.run_id == end.run_id && last.state == "completed")
                {
                    return Err(format!(
                        "listed method {method} did not complete its scripted run"
                    ));
                }
            }
        }
        _ => return Err(format!("unknown routed-account case {name}")),
    }
    Ok(Ending::Passed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peer() -> RecordingRoutingPeer {
        RecordingRoutingPeer::new(
            "apikey",
            json!({
                "provider": "provider-a", "account_id": "account-a", "credential_id": "key-a"
            }),
        )
        .unwrap()
    }

    #[test]
    fn account_route_peer_keeps_vendor_fixture_and_requires_route_names() {
        let route = AccountRoute::new("pool-owner", "pool-model");
        let peer = peer().with_account_route(route.clone());
        assert_eq!(peer.account_route(), Some(&route));
        assert_eq!(peer.fixture()["provider"], "provider-a");
        let body = json!({"provider":"pool-owner", "model":"pool-model",
            "conversation_key":"session-a", "admission_id":"admit-a"});
        let answer = peer
            .request("runner-a", "route.account_for", body.clone())
            .unwrap();
        assert_eq!(
            answer["result"]["account"],
            json!({"provider":"pool-owner",
            "account_id":"account-a", "credential_id":"key-a", "method":"apikey"})
        );
        for (field, vendor) in [("provider", "provider-a"), ("model", "model-a")] {
            let mut wrong = body.clone();
            wrong[field] = json!(vendor);
            assert!(peer
                .request("runner-a", "route.account_for", wrong)
                .unwrap_err()
                .to_string()
                .contains("did not forward account_route"));
        }
        assert_eq!(peer.calls().len(), 3);
    }

    #[test]
    fn routed_accounts_observation_replays_first_id_per_runner() {
        let peer = peer();
        let first_body =
            json!({"event_id":"event-a", "kind":"run_finished", "admission_id":"admission-a"});
        let first = peer
            .request("runner-a", "route.account_observe", first_body.clone())
            .unwrap();
        assert_eq!(
            first,
            json!({"result":{"observation":{
                "id":"observation-1", "duplicate":false, "bound":true, "effects":[]
            }}})
        );
        let repeated_body = json!({"event_id":"event-a", "kind":"account_limit_exhausted"});
        let repeated = peer
            .clone()
            .request("runner-a", "route.account_observe", repeated_body.clone())
            .unwrap();
        assert_eq!(
            repeated,
            json!({"result":{"observation":{
                "id":"observation-1", "duplicate":true, "bound":true, "effects":[]
            }}})
        );
        let other = peer
            .request("runner-b", "route.account_observe", first_body.clone())
            .unwrap();
        assert_eq!(other["result"]["observation"]["id"], "observation-2");
        assert_eq!(other["result"]["observation"]["duplicate"], false);
        let calls = peer.calls();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[0].body, first_body);
        assert_eq!(calls[1].body, repeated_body);
        assert_eq!(calls[2].runner, "runner-b");
    }

    #[test]
    fn routed_accounts_peer_admits_and_renews_fixture_account() {
        let peer = peer();
        let admit = json!({"provider":"provider-a", "model":"model-a", "conversation_key":"session-a", "admission_id":"admit-a", "decision_id":"caller-decision"});
        let first = peer
            .request("runner-a", "route.account_for", admit.clone())
            .unwrap();
        assert_eq!(
            first["result"]["admission"],
            json!({"id":"admit-a", "state":"admitted"})
        );
        assert_eq!(
            first["result"]["account"],
            json!({"provider":"provider-a", "account_id":"account-a", "credential_id":"key-a", "method":"apikey"})
        );
        assert_eq!(first["result"]["reason"]["decision_id"], "caller-decision");
        let mut retry = admit.clone();
        retry["mode"] = json!("admit");
        retry["service_tier"] = json!("standard");
        assert_eq!(
            peer.request("runner-a", "route.account_for", retry)
                .unwrap(),
            first
        );
        let pin = &first["result"]["pin"];
        let renewal = json!({"provider":"provider-a", "model":"model-a", "conversation_key":"session-a", "admission_id":"renew-a", "mode":"renew", "expected_pin":{"id":pin["id"], "generation":pin["generation"]}});
        let renewed = peer
            .request("runner-a", "route.account_for", renewal.clone())
            .unwrap();
        assert_eq!(renewed["result"]["account"], first["result"]["account"]);
        assert_eq!(renewed["result"]["pin"]["id"], pin["id"]);
        assert_eq!(renewed["result"]["pin"]["mode"], "renew");
        assert_eq!(renewed["result"]["admission"]["id"], "renew-a");
        assert!(
            renewed["result"]["pin"]["lease_valid_until_ms"].as_u64()
                > pin["lease_valid_until_ms"].as_u64()
        );
        assert_eq!(
            peer.request("runner-a", "route.account_for", renewal)
                .unwrap(),
            renewed
        );
        let mut conflicting = admit;
        conflicting["pool"] = json!("different-pool");
        assert!(peer
            .request("runner-a", "route.account_for", conflicting)
            .unwrap_err()
            .to_string()
            .contains("admission_conflict"));
    }
}
