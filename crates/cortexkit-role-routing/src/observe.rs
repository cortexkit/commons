//! `route.account_observe`: what a runner saw on an account, so the
//! provider's headroom stays current.
//!
//! Only runner modules call it. It is idempotent per calling runner and
//! `event_id`: a second observation with the same id is not applied again,
//! and its answer says `duplicate: true`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors::Refusal, members::Members};

open_set! {
    /// What the runner observed. An unknown kind is kept as advisory.
    pub enum ObserveKind {
        AdmissionStarted => "admission_started",
        RunFinished => "run_finished",
        Attempt => "attempt",
        ScopedNotFound => "scoped_not_found",
        AccountLimitExhausted => "account_limit_exhausted",
    }
}

/// The `route.account_observe` request.
/// Build with [`Self::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountObserveRequest {
    /// The idempotency key, unique per runner (a log position, say).
    pub event_id: String,
    pub kind: ObserveKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub admission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub served_account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_digest: Option<String>,
    /// The original observation time, never a forwarding time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_origin: Option<String>,
    /// The provider's rate-limit headers: names lowercased, raw values,
    /// unknown names included. Absent means none were received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_source: Option<String>,
    /// The quota pool for this reading or used-account report.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
}

impl AccountObserveRequest {
    pub fn new(event_id: impl Into<String>, kind: ObserveKind) -> Self {
        Self {
            event_id: event_id.into(),
            kind,
            admission_id: None,
            run_id: None,
            served_account_id: None,
            credential_id: None,
            view_digest: None,
            observed_at_ms: None,
            upstream_origin: None,
            headers: None,
            limit_window: None,
            resets_at_ms: None,
            resets_at_source: None,
            pool: None,
        }
    }

    /// Set the admission the observation belongs to.
    pub fn with_admission_id(mut self, admission_id: impl Into<String>) -> Self {
        self.admission_id = Some(admission_id.into());
        self
    }

    /// Set the run the observation belongs to.
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    /// Set the account that served the request.
    pub fn with_served_account_id(mut self, served_account_id: impl Into<String>) -> Self {
        self.served_account_id = Some(served_account_id.into());
        self
    }

    /// Set the public credential id.
    pub fn with_credential_id(mut self, credential_id: impl Into<String>) -> Self {
        self.credential_id = Some(credential_id.into());
        self
    }

    /// Set the digest of the runner's account view.
    pub fn with_view_digest(mut self, view_digest: impl Into<String>) -> Self {
        self.view_digest = Some(view_digest.into());
        self
    }

    /// Set the original observation time in Unix milliseconds.
    pub fn with_observed_at_ms(mut self, observed_at_ms: u64) -> Self {
        self.observed_at_ms = Some(observed_at_ms);
        self
    }

    /// Set the upstream origin of the observation.
    pub fn with_upstream_origin(mut self, upstream_origin: impl Into<String>) -> Self {
        self.upstream_origin = Some(upstream_origin.into());
        self
    }

    /// Set the provider's raw rate-limit headers, with lowercase names.
    pub fn with_headers(mut self, headers: BTreeMap<String, String>) -> Self {
        self.headers = Some(headers);
        self
    }

    /// Set the observed usage window.
    pub fn with_limit_window(mut self, limit_window: impl Into<String>) -> Self {
        self.limit_window = Some(limit_window.into());
        self
    }

    /// Set the usage window's reset time in Unix milliseconds.
    pub fn with_resets_at_ms(mut self, resets_at_ms: u64) -> Self {
        self.resets_at_ms = Some(resets_at_ms);
        self
    }

    /// Set the source of the usage window's reset time.
    pub fn with_resets_at_source(mut self, resets_at_source: impl Into<String>) -> Self {
        self.resets_at_source = Some(resets_at_source.into());
        self
    }

    /// Set the quota pool for the reading or used-account report.
    pub fn with_pool(mut self, pool: impl Into<String>) -> Self {
        self.pool = Some(pool.into());
        self
    }
}

/// What the provider did with an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    /// The runner sent this `event_id` before; nothing was applied again.
    pub duplicate: bool,
    /// The observation matched an admission.
    pub bound: bool,
    /// The changes the provider applied, in its own words: an account it
    /// stopped allocating for now, or a change to an account's remaining
    /// headroom.
    pub effects: Vec<String>,
}

/// The `route.account_observe` answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountObserveAnswer {
    pub observation: Observation,
}

/// Validate and decode `route.account_observe` params. A bad member is
/// `invalid_request` naming it; a header name that is not lowercase names
/// `headers`. A kind this crate does not name is accepted and kept.
pub fn parse_account_observe_request(params: &Value) -> Result<AccountObserveRequest, Refusal> {
    let members = Members::of(params)?;
    let headers: Option<BTreeMap<String, String>> = members.optional("headers")?;
    if let Some(name) = headers
        .iter()
        .flat_map(BTreeMap::keys)
        .find(|name| name.chars().any(char::is_uppercase))
    {
        return Err(members.refuse("headers", format!("header name {name:?} is not lowercase")));
    }
    Ok(AccountObserveRequest {
        event_id: members.text("event_id", None)?,
        kind: members.required("kind")?,
        admission_id: members.optional_text("admission_id", None)?,
        run_id: members.optional_text("run_id", None)?,
        served_account_id: members.optional_text("served_account_id", None)?,
        credential_id: members.optional_text("credential_id", None)?,
        view_digest: members.optional_text("view_digest", None)?,
        observed_at_ms: members.optional("observed_at_ms")?,
        upstream_origin: members.optional_text("upstream_origin", None)?,
        headers,
        limit_window: members.optional_text("limit_window", None)?,
        resets_at_ms: members.optional("resets_at_ms")?,
        resets_at_source: members.optional_text("resets_at_source", None)?,
        pool: members.pool()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_unknown_kind_is_kept() {
        let request =
            parse_account_observe_request(&json!({"event_id": "runner-log:7", "kind": "warmup"}))
                .unwrap();
        assert_eq!(request.kind, ObserveKind::Other("warmup".into()));
    }

    #[test]
    fn uppercase_header_names_are_refused() {
        let refusal = parse_account_observe_request(&json!({
            "event_id": "e",
            "kind": "attempt",
            "headers": {"X-RateLimit-Remaining": "4"}
        }))
        .unwrap_err();
        assert_eq!(refusal.detail["field"], "headers");
    }
}
