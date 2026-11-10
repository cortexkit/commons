//! `route.account_observe`: what a runner saw on an account, so the
//! provider's headroom stays current.
//!
//! Runner modules call it; with `observe_backoff`, a decision owner may also
//! report `account_limit_exhausted`, `transient_backoff`, or (with
//! `observe_credential_refused`) `credential_refused` on the admission
//! that carried its decision. It is idempotent
//! per calling principal and `event_id`: a second observation with the same
//! id returns the first observation's id, `duplicate: true` and empty effects.
//! The key is the principal and the event id, never the role: a runner and a
//! decision owner that are different principals don't collide, but one
//! principal acting in both roles must use a different event id for each
//! observation, or the second is a duplicate.
//!
//! `transient_backoff` fences the admission's account when `admission_id` is
//! sent, otherwise `served_account_id`. Until `retry_at_ms`, the router must
//! not allocate that account for new admissions on any runner. A past retry
//! time records evidence without fencing; a future time is capped at 24 hours
//! from receipt, as in [`crate::report::RESET_FENCE_CAP_MS`]. The existing pin
//! is not released by backoff; `run_finished` still releases it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    errors::{Refusal, RefusalCode},
    members::Members,
};

open_set! {
    /// What the runner observed. An unknown kind is kept as advisory.
    pub enum ObserveKind {
        AdmissionStarted => "admission_started",
        RunFinished => "run_finished",
        Attempt => "attempt",
        ScopedNotFound => "scoped_not_found",
        AccountLimitExhausted => "account_limit_exhausted",
        /// Account-local retry evidence distinct from quota exhaustion.
        TransientBackoff => "transient_backoff",
        /// A credential refused for every use, not a resettable usage limit.
        CredentialRefused => "credential_refused",
    }
}

/// Where the rate-limit headers in an observation came from. A closed set:
/// the router reads it to decide whether a header reading counts as evidence
/// that the account is exhausted, and refuses a value it doesn't name as
/// `invalid_request {field: "upstream_origin"}`. A new value needs a
/// describe group, because an older router refuses it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum UpstreamOrigin {
    /// The provider's own response headers, as the runner received them,
    /// whatever the transport. Use it when the runner can't say more.
    Upstream,
    /// The provider's response, received straight from its HTTP API.
    Direct,
    /// The provider's response, passed through unchanged by an HTTP relay or
    /// proxy in front of the provider.
    HttpRelayUpstream,
    /// The headers on a WebSocket stream's response-start frame.
    WsResponseStart,
    /// Headers a relay or wrapper made up itself, such as a placeholder sent
    /// before the real response. Never evidence.
    OptimisticWrapper,
}

impl UpstreamOrigin {
    /// Whether the router may count a header reading with this origin as
    /// evidence: every origin except [`Self::OptimisticWrapper`].
    pub fn is_genuine(self) -> bool {
        !matches!(self, Self::OptimisticWrapper)
    }
}

/// Where an observation's `resets_at_ms` came from. A closed set, refused by
/// field like [`UpstreamOrigin`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ResetsAtSource {
    /// The provider stated the window's reset time in a response header.
    Header,
    /// The provider stated an absolute reset time in a refusal body.
    Body,
    /// Worked out from a relative delay, such as a Retry-After value. Only a
    /// hint for when to try again, never the window's reset.
    RetryAfter,
}

impl ResetsAtSource {
    /// Whether the provider stated the reset itself, so the router may use it
    /// as the reset that ends a block: `header` and `body`, never
    /// `retry_after`.
    pub fn is_stated_reset(self) -> bool {
        matches!(self, Self::Header | Self::Body)
    }
}

/// The `route.account_observe` request.
/// Build with [`Self::new`] and the `with_*` setters.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountObserveRequest {
    /// The idempotency key, unique per calling principal (a log position, say).
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
    /// Where the observation's headers came from. Absent means header readings
    /// are recorded but can't confirm exhaustion.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upstream_origin: Option<UpstreamOrigin>,
    /// The provider's rate-limit headers: names lowercased, raw values,
    /// unknown names included. Absent means none were received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<BTreeMap<String, String>>,
    /// The quota window, or `credit` for an account-level spending or credit wall
    /// on a still-valid credential. Credit observations need no reset evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_window: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    /// Where `resets_at_ms` came from. Absent is read like
    /// [`ResetsAtSource::RetryAfter`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_source: Option<ResetsAtSource>,
    /// The quota pool this observation is about: the pool a reading describes,
    /// or the pool an `account_limit_exhausted` observation ran out of.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pool: Option<String>,
    /// Informative for runners; for decision owners, the select or panel
    /// decision carried by the named admission. Requires `observe_backoff`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    /// Unix ms; required with `transient_backoff`, allowed with
    /// `account_limit_exhausted`, and refused with other kinds, including
    /// `credential_refused`.
    /// Requires `observe_backoff`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_at_ms: Option<u64>,
    /// Verbatim provider code, 1–128 characters. Requires `observe_backoff`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_code: Option<String>,
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
            decision_id: None,
            retry_at_ms: None,
            provider_code: None,
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
    pub fn with_upstream_origin(mut self, upstream_origin: UpstreamOrigin) -> Self {
        self.upstream_origin = Some(upstream_origin);
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
    pub fn with_resets_at_source(mut self, resets_at_source: ResetsAtSource) -> Self {
        self.resets_at_source = Some(resets_at_source);
        self
    }

    /// Set the quota pool this observation is about.
    pub fn with_pool(mut self, pool: impl Into<String>) -> Self {
        self.pool = Some(pool.into());
        self
    }

    pub fn with_decision_id(mut self, decision_id: impl Into<String>) -> Self {
        self.decision_id = Some(decision_id.into());
        self
    }

    pub fn with_retry_at_ms(mut self, retry_at_ms: u64) -> Self {
        self.retry_at_ms = Some(retry_at_ms);
        self
    }

    pub fn with_provider_code(mut self, provider_code: impl Into<String>) -> Self {
        self.provider_code = Some(provider_code.into());
        self
    }
}

/// Authorization evidence resolved by the router from its own state.
/// Used only by routers declaring `observe_backoff`; other routers retain the
/// runner-only authorization rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ObserveArm<'a> {
    /// The caller holds the admission or satisfies the runner authorization
    /// rule. This is determined by admission binding, not decision ownership.
    Runner,
    /// The caller owns a retained select or panel decision. `admission_id` is
    /// the admission the router looked up and `carried` is the decision that
    /// admission carried, if any. Both come from router state, never from the
    /// request. Checking the id makes a wrong-admission lookup fail closed.
    #[non_exhaustive]
    DecisionOwner {
        decision_id: &'a str,
        admission_id: &'a str,
        carried: Option<&'a str>,
    },
    /// Neither an authorized runner nor the owner of a select/panel decision.
    /// Includes foreign and non-select decisions sent by non-runners.
    Other,
}

impl<'a> ObserveArm<'a> {
    pub fn runner() -> Self {
        Self::Runner
    }

    pub fn decision_owner(
        decision_id: &'a str,
        admission_id: &'a str,
        carried: Option<&'a str>,
    ) -> Self {
        Self::DecisionOwner {
            decision_id,
            admission_id,
            carried,
        }
    }

    pub fn other() -> Self {
        Self::Other
    }
}

/// Check authorization after the router resolves the caller's arm.
/// Runners may send any kind and any informative `decision_id`, including a
/// foreign decision or an `account_continue` offer the admission never carried.
/// Decision owners may send only the kinds that stop the router allocating an
/// account or credential: `account_limit_exhausted`, `transient_backoff` or
/// `credential_refused`. They can never release or reshape the runner's
/// account pin.
/// Every denial is `account_not_authorized`, not `unknown_decision`.
pub fn check_observe_authorization(
    arm: ObserveArm<'_>,
    request: &AccountObserveRequest,
) -> Result<(), Refusal> {
    let allowed = match arm {
        ObserveArm::Runner => true,
        ObserveArm::DecisionOwner {
            decision_id,
            admission_id,
            carried,
        } => {
            matches!(
                request.kind,
                ObserveKind::AccountLimitExhausted
                    | ObserveKind::TransientBackoff
                    | ObserveKind::CredentialRefused
            ) && request.admission_id.as_deref() == Some(admission_id)
                && request.decision_id.as_deref() == Some(decision_id)
                && carried == Some(decision_id)
        }
        ObserveArm::Other => false,
    };
    if allowed {
        Ok(())
    } else {
        Err(Refusal::new(
            RefusalCode::AccountNotAuthorized,
            "caller may not observe this account",
        ))
    }
}

/// What the provider did with an observation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Observation {
    pub id: String,
    /// This principal sent this `event_id` before; nothing was applied again.
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
    parse_account_observe_request_serving::<&str>(params, &[])
}

/// Validate and decode observations for the router's declared groups.
/// Without `observe_backoff`, the added members are refused by name and
/// `transient_backoff` remains an advisory unknown kind with no retry rule.
/// Without `observe_credential_refused`, `credential_refused` is advisory too.
/// Null optional members are treated as absent.
pub fn parse_account_observe_request_serving<C: AsRef<str>>(
    params: &Value,
    capabilities: &[C],
) -> Result<AccountObserveRequest, Refusal> {
    let members = Members::of(params)?;
    let backoff = capabilities
        .iter()
        .any(|cap| cap.as_ref() == "observe_backoff");
    let credential_refused = capabilities
        .iter()
        .any(|cap| cap.as_ref() == "observe_credential_refused");
    if !backoff {
        for field in ["decision_id", "retry_at_ms", "provider_code"] {
            if members
                .object
                .get(field)
                .is_some_and(|value| !value.is_null())
            {
                return Err(members.refuse(field, "member requires observe_backoff"));
            }
        }
    }
    let mut kind: ObserveKind = members.required("kind")?;
    if !backoff && kind == ObserveKind::TransientBackoff {
        kind = ObserveKind::Other("transient_backoff".into());
    }
    if !credential_refused && kind == ObserveKind::CredentialRefused {
        kind = ObserveKind::Other("credential_refused".into());
    }
    let headers: Option<BTreeMap<String, String>> = members.optional("headers")?;
    if let Some(name) = headers
        .iter()
        .flat_map(BTreeMap::keys)
        .find(|name| name.chars().any(char::is_uppercase))
    {
        return Err(members.refuse("headers", format!("header name {name:?} is not lowercase")));
    }
    let request = AccountObserveRequest {
        event_id: members.text("event_id", None)?,
        kind,
        admission_id: members.optional_text("admission_id", None)?,
        run_id: members.optional_text("run_id", None)?,
        served_account_id: members.optional_text("served_account_id", None)?,
        credential_id: members.optional_text("credential_id", None)?,
        view_digest: members.optional_text("view_digest", None)?,
        observed_at_ms: members.optional("observed_at_ms")?,
        upstream_origin: members.optional("upstream_origin")?,
        headers,
        limit_window: members.optional_text("limit_window", None)?,
        resets_at_ms: members.optional("resets_at_ms")?,
        resets_at_source: members.optional("resets_at_source")?,
        pool: members.pool()?,
        decision_id: members.optional_text("decision_id", None)?,
        retry_at_ms: members.optional("retry_at_ms")?,
        provider_code: members.optional_text("provider_code", Some(128))?,
    };
    if request.kind == ObserveKind::CredentialRefused
        && request.credential_id.is_none()
        && request.admission_id.is_none()
    {
        return Err(members.refuse(
            "admission_id",
            "credential_refused requires a credential or admission",
        ));
    }
    if backoff {
        match request.kind {
            ObserveKind::TransientBackoff => {
                if request.retry_at_ms.is_none() {
                    return Err(
                        members.refuse("retry_at_ms", "transient_backoff requires retry_at_ms")
                    );
                }
                if request.admission_id.is_none() && request.served_account_id.is_none() {
                    return Err(members.refuse(
                        "admission_id",
                        "transient_backoff requires an admission or served account",
                    ));
                }
            }
            ObserveKind::AccountLimitExhausted => {}
            _ if request.retry_at_ms.is_some() => {
                return Err(members.refuse(
                    "retry_at_ms",
                    "retry_at_ms requires transient_backoff or account_limit_exhausted",
                ));
            }
            _ => {}
        }
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn backoff() -> Value {
        json!({"event_id": "e", "kind": "transient_backoff", "retry_at_ms": 42, "admission_id": "a"})
    }

    fn parse_backoff(params: &Value) -> Result<AccountObserveRequest, Refusal> {
        parse_account_observe_request_serving(params, &["observe_backoff"])
    }

    fn assert_invalid(result: Result<AccountObserveRequest, Refusal>, field: &str) {
        let refusal = result.unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], field);
    }

    #[test]
    fn account_limit_exhausted_credit_accepts_no_reset_fields() {
        let params = json!({
            "event_id": "credit-wall-1",
            "kind": "account_limit_exhausted",
            "served_account_id": "account-copper",
            "credential_id": "credential-copper",
            "limit_window": "credit"
        });
        for groups in [&[][..], &["observe_backoff"][..]] {
            let request = parse_account_observe_request_serving(&params, groups).unwrap();
            assert_eq!(request.kind, ObserveKind::AccountLimitExhausted);
            assert_eq!(request.limit_window.as_deref(), Some("credit"));
            assert_eq!(request.resets_at_ms, None);
            assert_eq!(request.resets_at_source, None);
            assert_eq!(request.retry_at_ms, None);
            assert_eq!(serde_json::to_value(request).unwrap(), params);
        }
        assert_eq!(
            serde_json::to_value(parse_account_observe_request(&params).unwrap()).unwrap(),
            params
        );
    }

    #[test]
    fn account_limit_exhausted_quota_reset_evidence_stays_optional() {
        for window in ["five_hour", "seven_day"] {
            for reset in [None, Some(1_900_000_000_000_u64)] {
                let mut params = json!({
                    "event_id": "quota-wall-1",
                    "kind": "account_limit_exhausted",
                    "limit_window": window
                });
                if let Some(reset) = reset {
                    params["resets_at_ms"] = json!(reset);
                }
                let request = parse_account_observe_request(&params).unwrap();
                assert_eq!(request.limit_window.as_deref(), Some(window));
                assert_eq!(request.resets_at_ms, reset);
                assert_eq!(request.resets_at_source, None);
            }
        }
    }

    #[test]
    fn added_observe_members_require_observe_backoff_by_name() {
        for (field, value) in [
            ("decision_id", json!("d")),
            ("retry_at_ms", json!(42)),
            ("provider_code", json!("please_retry")),
        ] {
            for value in [value, json!({"malformed": true})] {
                let mut params = json!({"event_id": "e", "kind": "attempt"});
                params[field] = value;
                for groups in [&[][..], &["unrelated"][..]] {
                    assert_invalid(
                        parse_account_observe_request_serving(&params, groups),
                        field,
                    );
                }
                assert_invalid(parse_account_observe_request(&params), field);
            }
        }
    }

    #[test]
    fn undeclared_transient_backoff_is_advisory_without_retry_rules() {
        let params = json!({"event_id": "e", "kind": "transient_backoff"});
        let request = parse_account_observe_request_serving::<&str>(&params, &[]).unwrap();
        assert_eq!(request.kind, ObserveKind::Other("transient_backoff".into()));
        assert_eq!(serde_json::to_value(request).unwrap(), params);
        let unknown = parse_backoff(&json!({"event_id": "e", "kind": "future_kind"})).unwrap();
        assert_eq!(unknown.kind, ObserveKind::Other("future_kind".into()));
    }

    fn parse_credential_refused(params: &Value) -> Result<AccountObserveRequest, Refusal> {
        parse_account_observe_request_serving(
            params,
            &["observe_backoff", "observe_credential_refused"],
        )
    }

    #[test]
    fn credential_refused_decodes_only_with_its_group() {
        let params = json!({"event_id": "e", "kind": "credential_refused", "credential_id": "c"});
        let request = parse_credential_refused(&params).unwrap();
        assert_eq!(request.kind, ObserveKind::CredentialRefused);
        assert_eq!(serde_json::to_value(request).unwrap(), params);
        let advisory = json!({"event_id": "e", "kind": "credential_refused"});
        for groups in [&[][..], &["observe_backoff"][..], &["unrelated"][..]] {
            let request = parse_account_observe_request_serving(&advisory, groups).unwrap();
            assert_eq!(
                request.kind,
                ObserveKind::Other("credential_refused".into())
            );
            assert_eq!(serde_json::to_value(request).unwrap(), advisory);
        }
        assert_eq!(
            parse_account_observe_request(&advisory).unwrap().kind,
            ObserveKind::Other("credential_refused".into())
        );
    }

    #[test]
    fn credential_refused_requires_a_credential_or_admission_and_refuses_retry_time() {
        let mut params = json!({"event_id": "e", "kind": "credential_refused"});
        assert_invalid(parse_credential_refused(&params), "admission_id");
        params["served_account_id"] = json!("account");
        assert_invalid(parse_credential_refused(&params), "admission_id");
        for field in ["credential_id", "admission_id"] {
            params[field] = json!("id");
            assert!(parse_credential_refused(&params).is_ok());
            params["retry_at_ms"] = json!(42);
            assert_invalid(parse_credential_refused(&params), "retry_at_ms");
            params["retry_at_ms"] = Value::Null;
            assert!(parse_credential_refused(&params).is_ok());
            params[field] = Value::Null;
        }
    }

    #[test]
    fn credential_refused_keeps_provider_code_verbatim_with_existing_bounds() {
        let mut params =
            json!({"event_id": "e", "kind": "credential_refused", "admission_id": "a"});
        for code in [" Billing-Empty\n".into(), "é".repeat(128)] {
            params["provider_code"] = json!(code);
            assert_eq!(
                parse_credential_refused(&params)
                    .unwrap()
                    .provider_code
                    .as_deref(),
                Some(code.as_str())
            );
        }
        for code in [json!(""), json!("é".repeat(129)), json!(7)] {
            params["provider_code"] = code;
            assert_invalid(parse_credential_refused(&params), "provider_code");
        }
    }

    #[test]
    fn transient_backoff_requires_retry_time_and_a_fenced_account() {
        let request = parse_backoff(&backoff()).unwrap();
        assert_eq!(request.kind, ObserveKind::TransientBackoff);
        assert_eq!(request.retry_at_ms, Some(42));
        assert_eq!(request.admission_id.as_deref(), Some("a"));
        let mut params = backoff();
        params.as_object_mut().unwrap().remove("admission_id");
        assert_invalid(parse_backoff(&params), "admission_id");
        params["served_account_id"] = json!("account");
        assert_eq!(
            parse_backoff(&params).unwrap().served_account_id.as_deref(),
            Some("account")
        );
        params["admission_id"] = json!("a");
        assert!(parse_backoff(&params).is_ok());
        params.as_object_mut().unwrap().remove("retry_at_ms");
        assert_invalid(parse_backoff(&params), "retry_at_ms");
        params["retry_at_ms"] = Value::Null;
        assert_invalid(parse_backoff(&params), "retry_at_ms");
    }

    #[test]
    fn retry_time_is_unsigned_and_only_allowed_for_resettable_wall_kinds() {
        for kind in ["transient_backoff", "account_limit_exhausted"] {
            for retry_at_ms in [json!(0), json!(u64::MAX)] {
                let mut params = backoff();
                params["kind"] = json!(kind);
                params["retry_at_ms"] = retry_at_ms.clone();
                assert_eq!(
                    serde_json::to_value(parse_backoff(&params).unwrap()).unwrap()["retry_at_ms"],
                    retry_at_ms
                );
            }
            for retry_at_ms in [json!(-1), json!(1.5), json!("42"), json!(false)] {
                let mut params = backoff();
                params["kind"] = json!(kind);
                params["retry_at_ms"] = retry_at_ms;
                assert_invalid(parse_backoff(&params), "retry_at_ms");
            }
        }
        assert!(
            parse_backoff(&json!({"event_id": "e", "kind": "account_limit_exhausted"})).is_ok()
        );
        for kind in [
            "admission_started",
            "run_finished",
            "attempt",
            "scoped_not_found",
            "future_kind",
        ] {
            let mut params = backoff();
            params["kind"] = json!(kind);
            assert_invalid(parse_backoff(&params), "retry_at_ms");
            params["retry_at_ms"] = Value::Null;
            assert!(parse_backoff(&params).is_ok());
        }
    }

    #[test]
    fn observe_optional_member_bounds_and_verbatim_provider_code() {
        for code in ["x".into(), "é".repeat(128), " Rate-Limit\n".into()] {
            let mut params = backoff();
            params["provider_code"] = json!(code);
            params["decision_id"] = json!("d");
            let request = parse_backoff(&params).unwrap();
            assert_eq!(request.provider_code.as_deref(), Some(code.as_str()));
            assert_eq!(request.decision_id.as_deref(), Some("d"));
        }
        for (field, value) in [
            ("provider_code", json!("")),
            ("provider_code", json!("é".repeat(129))),
            ("provider_code", json!(7)),
            ("decision_id", json!("")),
            ("decision_id", json!(false)),
        ] {
            let mut params = backoff();
            params[field] = value;
            assert_invalid(parse_backoff(&params), field);
        }
        let request = AccountObserveRequest::new("e", ObserveKind::TransientBackoff)
            .with_admission_id("a")
            .with_decision_id("d")
            .with_retry_at_ms(42)
            .with_provider_code("retry");
        let params = serde_json::to_value(&request).unwrap();
        assert_eq!(params["decision_id"], "d");
        assert_eq!(params["retry_at_ms"], 42);
        assert_eq!(params["provider_code"], "retry");
        assert_eq!(parse_backoff(&params).unwrap(), request);
    }

    fn owner_wall(kind: ObserveKind) -> AccountObserveRequest {
        AccountObserveRequest::new("e", kind)
            .with_admission_id("a")
            .with_decision_id("d")
    }

    fn assert_unauthorized(arm: ObserveArm<'_>, request: &AccountObserveRequest) {
        assert_eq!(
            check_observe_authorization(arm, request).unwrap_err().code,
            RefusalCode::AccountNotAuthorized
        );
    }

    #[test]
    fn decision_owner_can_report_only_walls_on_the_admission_carrying_its_decision() {
        for kind in [
            ObserveKind::AccountLimitExhausted,
            ObserveKind::TransientBackoff,
            ObserveKind::CredentialRefused,
        ] {
            let request = owner_wall(kind);
            let owner = ObserveArm::decision_owner("d", "a", Some("d"));
            assert!(check_observe_authorization(owner, &request).is_ok());
            assert_unauthorized(
                ObserveArm::decision_owner("d", "a", Some("different")),
                &request,
            );
            assert_unauthorized(ObserveArm::decision_owner("d", "a", None), &request);
            assert_unauthorized(
                ObserveArm::decision_owner("d", "another_admission", Some("d")),
                &request,
            );
            let mut changed = request.clone();
            changed.admission_id = None;
            assert_unauthorized(owner, &changed);
            changed = request.clone();
            changed.decision_id = None;
            assert_unauthorized(owner, &changed);
            changed.decision_id = Some("different".into());
            assert_unauthorized(owner, &changed);
        }
    }

    #[test]
    fn decision_owner_cannot_finish_or_reshape_runner_pins() {
        for kind in [
            ObserveKind::RunFinished,
            ObserveKind::AdmissionStarted,
            ObserveKind::Attempt,
            ObserveKind::ScopedNotFound,
            ObserveKind::Other("future_kind".into()),
        ] {
            assert_unauthorized(
                ObserveArm::decision_owner("d", "a", Some("d")),
                &owner_wall(kind),
            );
        }
    }

    #[test]
    fn other_arm_refuses_foreign_and_non_select_decisions() {
        for decision_id in [
            "foreign_select_decision",
            "account_continue_offer",
            "account_for_reason",
        ] {
            for kind in [
                ObserveKind::RunFinished,
                ObserveKind::TransientBackoff,
                ObserveKind::AccountLimitExhausted,
                ObserveKind::CredentialRefused,
            ] {
                let request = owner_wall(kind).with_decision_id(decision_id);
                assert_unauthorized(ObserveArm::other(), &request);
            }
        }
    }

    #[test]
    fn runner_arm_allows_foreign_and_account_continue_offer_decisions() {
        for decision_id in ["foreign_select_decision", "account_continue_offer"] {
            for kind in [
                ObserveKind::RunFinished,
                ObserveKind::TransientBackoff,
                ObserveKind::AccountLimitExhausted,
                ObserveKind::CredentialRefused,
                ObserveKind::AdmissionStarted,
                ObserveKind::Attempt,
                ObserveKind::ScopedNotFound,
                ObserveKind::Other("future_kind".into()),
            ] {
                let request = owner_wall(kind).with_decision_id(decision_id);
                assert!(check_observe_authorization(ObserveArm::runner(), &request).is_ok());
            }
        }
        let request = AccountObserveRequest::new("e", ObserveKind::Attempt);
        assert!(check_observe_authorization(ObserveArm::runner(), &request).is_ok());
    }

    #[test]
    fn legacy_observe_parser_is_serving_without_groups() {
        for params in [
            json!({"event_id": "e", "kind": "attempt"}),
            json!({"event_id": "e", "kind": "transient_backoff"}),
            json!({"event_id": "e", "kind": "credential_refused"}),
            backoff(),
            json!({"event_id": "e", "kind": "future_kind"}),
            json!({"event_id": "e", "kind": 7}),
            json!({"kind": "attempt"}),
            Value::Null,
        ] {
            assert_eq!(
                parse_account_observe_request(&params),
                parse_account_observe_request_serving::<&str>(&params, &[])
            );
        }
    }

    #[test]
    fn observe_0_6_0_bytes_round_trip_and_absent_members_are_omitted() {
        for body in [
            r#"{"event_id":"e","kind":"attempt"}"#,
            r#"{"event_id":"e","kind":"account_limit_exhausted","admission_id":"a","run_id":"r","served_account_id":"account","credential_id":"cred","view_digest":"view","observed_at_ms":7,"upstream_origin":"direct","headers":{"x-remaining":"0"},"limit_window":"daily","resets_at_ms":42,"resets_at_source":"header","pool":"default"}"#,
        ] {
            let params: Value = serde_json::from_str(body).unwrap();
            let request = parse_account_observe_request(&params).unwrap();
            assert_eq!(serde_json::to_string(&request).unwrap(), body);
            assert_eq!(parse_backoff(&params).unwrap(), request);
        }
        let request = AccountObserveRequest::new("e", ObserveKind::Attempt);
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"event_id":"e","kind":"attempt"}"#
        );
        let params = json!({"event_id": "e", "kind": "attempt", "decision_id": null, "retry_at_ms": null, "provider_code": null});
        assert_eq!(parse_account_observe_request(&params).unwrap(), request);
        assert_eq!(parse_backoff(&params).unwrap(), request);
    }

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

    #[test]
    fn every_named_origin_and_reset_source_round_trips() {
        for (origin, text, genuine) in [
            (UpstreamOrigin::Upstream, "upstream", true),
            (UpstreamOrigin::Direct, "direct", true),
            (
                UpstreamOrigin::HttpRelayUpstream,
                "http_relay_upstream",
                true,
            ),
            (UpstreamOrigin::WsResponseStart, "ws_response_start", true),
            (
                UpstreamOrigin::OptimisticWrapper,
                "optimistic_wrapper",
                false,
            ),
        ] {
            assert_eq!(serde_json::to_value(origin).unwrap(), Value::from(text));
            assert_eq!(origin.is_genuine(), genuine, "{text}");
            let params =
                serde_json::json!({"event_id": "e", "kind": "attempt", "upstream_origin": text});
            assert_eq!(
                parse_account_observe_request(&params)
                    .unwrap()
                    .upstream_origin,
                Some(origin)
            );
        }
        for (source, text, stated) in [
            (ResetsAtSource::Header, "header", true),
            (ResetsAtSource::Body, "body", true),
            (ResetsAtSource::RetryAfter, "retry_after", false),
        ] {
            assert_eq!(serde_json::to_value(source).unwrap(), Value::from(text));
            assert_eq!(source.is_stated_reset(), stated, "{text}");
            let params =
                serde_json::json!({"event_id": "e", "kind": "attempt", "resets_at_source": text});
            assert_eq!(
                parse_account_observe_request(&params)
                    .unwrap()
                    .resets_at_source,
                Some(source)
            );
        }
    }

    #[test]
    fn an_unnamed_origin_or_reset_source_is_refused_by_field() {
        for (field, value) in [
            ("upstream_origin", "https://api.example.invalid"),
            ("upstream_origin", ""),
            ("upstream_origin", "Direct"),
            ("resets_at_source", "headers"),
            ("resets_at_source", "provider"),
        ] {
            let mut params = serde_json::json!({"event_id": "e", "kind": "attempt"});
            params[field] = Value::from(value);
            let refusal = parse_account_observe_request(&params).unwrap_err();
            assert_eq!(refusal.code, RefusalCode::InvalidRequest, "{field}={value}");
            assert_eq!(refusal.detail["field"], field, "{field}={value}");
        }
    }
}
