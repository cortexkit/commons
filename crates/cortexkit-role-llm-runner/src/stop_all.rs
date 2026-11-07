//! Durable, session-wide stop requests. Ordinary Stop still ends only a turn.
use crate::delivery_order::PlanSender;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// One total budget, including durable turn termination and parallel fan-out.
pub const BUDGET_MS: u64 = 15_000;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StopAllRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    pub request_id: String,
}
impl StopAllRequest {
    pub fn new(request_id: impl Into<String>) -> Self {
        Self {
            run_id: None,
            request_id: request_id.into(),
        }
    }
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }
}

/// Validate before admission, durable replay lookup or writes. The returned
/// string is the exact `invalid_params` field; session addressing is route-only.
pub fn check_request(params: &Map<String, Value>) -> Result<StopAllRequest, String> {
    for key in params.keys() {
        if !matches!(key.as_str(), "run_id" | "request_id") {
            return Err(key.clone());
        }
    }
    let id = params
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or("request_id")?;
    if !(1..=128).contains(&id.len()) || !id.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        return Err("request_id".into());
    }
    if params
        .get("run_id")
        .is_some_and(|run| run.as_str().is_none_or(str::is_empty))
    {
        return Err("run_id".into());
    }
    serde_json::from_value(Value::Object(params.clone())).map_err(|_| "request_id".into())
}

/// Use the recorded owner test from cancellation and the current plan grant.
/// Match the route's attested principal, never any request-body identity.
pub fn caller_is_admitted(
    is_owner: bool,
    attested_principal: &str,
    senders: &[PlanSender],
) -> bool {
    is_owner
        || senders
            .iter()
            .any(|sender| sender.module == attested_principal && sender.interrupt)
}

pub mod outcomes {
    pub const ANSWERED: &str = "answered";
    pub const NO_ANSWER: &str = "no_answer";
    pub const NOT_DECLARED: &str = "not_declared";
    pub const UNREACHABLE: &str = "unreachable";
}

/// Provider-owned items remain opaque JSON so forwarding preserves unknown
/// kinds, outcomes and members without reinterpreting their values.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ProviderStop {
    pub provider: String,
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more: Option<u64>,
}
impl ProviderStop {
    pub fn new(provider: impl Into<String>, outcome: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            outcome: outcome.into(),
            items: None,
            more: None,
        }
    }
    pub fn with_items(mut self, items: Vec<Value>) -> Self {
        self.items = Some(items);
        self
    }
    pub fn with_more(mut self, more: u64) -> Self {
        self.more = Some(more);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RunnerStop {
    pub items: Vec<Value>,
}
impl RunnerStop {
    pub fn new(items: Vec<Value>) -> Self {
        Self { items }
    }
}

/// Store the complete first reply under request_id before acknowledging it.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct StopAllReply {
    pub providers: Vec<ProviderStop>,
    pub runner: RunnerStop,
}
impl StopAllReply {
    pub fn new(providers: Vec<ProviderStop>, runner: RunnerStop) -> Self {
        Self { providers, runner }
    }
}
