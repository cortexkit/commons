//! In-process recording peers. Adapters forward their routing/vendor traffic here.

use crate::subject::{SubjectError, SubjectResult};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Default)]
struct RoutingState {
    now: u64,
    calls: Vec<(String, Value)>,
    refuse_next: Option<Value>,
    refuse_renewals: bool,
    pins: BTreeMap<String, Value>,
    admission_ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct RecordingRoutingPeer(Arc<Mutex<RoutingState>>);
impl RecordingRoutingPeer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn now_ms(&self) -> u64 {
        self.0.lock().unwrap().now
    }
    /// The adapter's clock hook advances this same clock before firing timers.
    pub fn advance_clock(&self, ms: u64) {
        self.0.lock().unwrap().now += ms;
    }
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.0.lock().unwrap().calls.clone()
    }
    pub fn refuse_next(&self, refusal: Value) {
        self.0.lock().unwrap().refuse_next = Some(refusal);
    }
    pub fn refuse_renewals(&self, value: bool) {
        self.0.lock().unwrap().refuse_renewals = value;
    }
    /// Change the validity duration of subsequent admissions and renewals.
    pub fn set_admission_ttl_ms(&self, ms: u64) {
        assert!(ms > 0, "admission TTL must be positive");
        self.0.lock().unwrap().admission_ttl_ms = Some(ms);
    }
    pub fn call(&self, method: &str, params: Value) -> SubjectResult<Value> {
        let mut state = self.0.lock().unwrap();
        state.calls.push((method.into(), params.clone()));
        if method == "route.account_observe" {
            return Ok(
                json!({"observation":{"id":params["event_id"],"duplicate":false,"bound":true,"effects":[]}}),
            );
        }
        if method != "route.account_for" {
            return Err(SubjectError::new("unknown routing method"));
        }
        if params["mode"] == "renew" && state.refuse_renewals {
            return Err(SubjectError::refused(
                json!({"code":"account_hold", "message":"fixture hold", "detail":{"until_ms": state.now + 1_000}}),
            ));
        }
        if let Some(refusal) = state.refuse_next.take() {
            return Err(SubjectError::refused(refusal));
        }
        let id = params["admission_id"]
            .as_str()
            .ok_or_else(|| SubjectError::new("missing admission_id"))?;
        let key = params["conversation_key"]
            .as_str()
            .ok_or_else(|| SubjectError::new("missing conversation_key"))?;
        if params["mode"] == "renew" {
            let previous = state
                .pins
                .get(key)
                .ok_or_else(|| SubjectError::new("renewal has no pin"))?;
            if params["expected_pin"]
                != json!({"id":previous["id"],"generation":previous["generation"]})
            {
                return Err(SubjectError::new(
                    "renewal expected_pin is not the latest pin",
                ));
            }
        }
        let ttl = state.admission_ttl_ms.unwrap_or(1_000);
        let pin = json!({"id":format!("pin:{key}"),"generation":state.calls.len(),
            "affinity_expires_at_ms":state.now + 10_000,"admission_valid_until_ms":state.now + ttl,
            "lease_valid_until_ms":state.now + ttl,"mode":params.get("mode").cloned().unwrap_or(json!("admit"))});
        state.pins.insert(key.into(), pin.clone());
        Ok(json!({"admission":{"id":id,"state":"admitted"},
            "account":{"provider":params["provider"],"account_id":"fixture-account","credential_id":"fixture-credential","method":"api_key"},
            "pin":pin,"reason":{"code":"decision","decision_id":params.get("decision_id").cloned().unwrap_or(json!("allocation")),"policy_version":"fixture","quota_age_band":"fresh"}}))
    }
}

#[derive(Debug, Default)]
struct VendorState {
    sessions: BTreeMap<String, (String, Value)>,
    tool_calls: BTreeMap<String, usize>,
}
/// Synthetic sessions only; no real token is stored or printed.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct RecordingVendor(Arc<Mutex<VendorState>>);
impl RecordingVendor {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn open_session(&self, session: &str, token_ref: &str, usage: Value) {
        self.0
            .lock()
            .unwrap()
            .sessions
            .insert(session.into(), (token_ref.into(), usage));
    }
    pub fn session(&self, session: &str) -> Option<(String, Value)> {
        self.0.lock().unwrap().sessions.get(session).cloned()
    }
    /// Queue a synthetic vendor tool-call notification for an open session.
    pub fn push_tool_call(&self, session: &str) -> SubjectResult<()> {
        let mut state = self.0.lock().unwrap();
        if !state.sessions.contains_key(session) {
            return Err(SubjectError::new("push_tool_call: unknown session"));
        }
        *state.tool_calls.entry(session.into()).or_default() += 1;
        Ok(())
    }
    /// Drain notifications for forwarding through the provider's watch transport.
    pub fn take_tool_calls(&self, session: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .tool_calls
            .remove(session)
            .unwrap_or(0)
    }
}
