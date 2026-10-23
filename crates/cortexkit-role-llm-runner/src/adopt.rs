//! `session.adopt`: the owner takes over a session the host started.
//!
//! A harness runner serves sessions a person starts in their own app. The
//! host fixes the tool set and system text when it starts, so the runner
//! applies the installation's fetch plan before the first turn, and the
//! owner has no prompt to send. `session.adopt` makes the caller the
//! session's owner and binds the session's frozen scope to that plan, as
//! owner admission records a plan. It starts no run, writes no message, and
//! neither interrupts nor refuses a run already in progress. A runner serves
//! it only when `role.describe` declares `adopt`
//! ([`crate::capabilities::ADOPT`]).
//!
//! The runner checks a request in this order, and the first failing check
//! answers:
//!
//! 1. the caller: only a caller the runner would record as the session's
//!    owner on an owner send is admitted, and once an owner is recorded,
//!    only that owner (`scope_owner_mismatch`);
//! 2. the `send_id`: an exact repeat replays the original reply, and the
//!    same `send_id` with another plan is refused `send_id_reuse` naming
//!    `plan` ([`AdoptRequest::reused_field`]);
//! 3. the plan: its identity must equal the identity of the plan the runner
//!    applied when the host started the session ([`check_drift`]), or the
//!    request is refused `plan_drift`.
//!
//! A refused request writes nothing, its `send_id` included.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::errors::PlanDriftDetail;

/// The `field` a `send_id_reuse` refusal names when a `send_id` is reused
/// with another plan.
pub const PLAN_FIELD: &str = "plan";

/// The `session.adopt` request: `{send_id, plan}`.
///
/// Strict: an unknown field is refused `invalid_params` naming it. In
/// particular the request never names its session: the session is the one
/// the route is bound to.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`AdoptRequest::new`], or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct AdoptRequest {
    /// The idempotency key: the runner keys adoptions by the bound session
    /// and this value. The owner derives it from the session and plan it
    /// adopts, so a retry after a lost reply replays instead of failing; never
    /// from the attempt (a counter or a time), and never from a `send_id` it
    /// used for a `session.send`.
    pub send_id: String,
    /// The full fetch plan the caller expects the session to run under: the
    /// same object `SendRequest::plan` carries, never a digest of it.
    #[serde(deserialize_with = "deserialize_plan")]
    pub plan: Map<String, Value>,
}

/// Decode `plan` as an object, refusing anything else (a digest string, say)
/// with a message that names the field.
fn deserialize_plan<'de, D>(deserializer: D) -> Result<Map<String, Value>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Value::deserialize(deserializer)? {
        Value::Object(plan) => Ok(plan),
        other => Err(serde::de::Error::custom(format!(
            "{PLAN_FIELD} must be the fetch-plan object; got {other}"
        ))),
    }
}

impl AdoptRequest {
    /// Check output-reference plan members before any adoption write.
    pub fn check_output_refs(&self, advertised: bool) -> Result<(), &'static str> {
        crate::output_refs::check_plan(&Value::Object(self.plan.clone()), advertised)
    }
    pub fn new(send_id: impl Into<String>, plan: Map<String, Value>) -> Self {
        Self {
            send_id: send_id.into(),
            plan,
        }
    }

    /// Compare `self` with an `earlier` request the runner already answered
    /// under the same `send_id`. `None` means an exact repeat, answered with
    /// the earlier reply. `Some(field)` names the field a `send_id_reuse`
    /// refusal reports. The comparison is of the whole request, as for owner
    /// sends; with the `send_id` equal, only the plan can differ.
    pub fn reused_field(&self, earlier: &Self) -> Option<&'static str> {
        (self != earlier).then_some(PLAN_FIELD)
    }
}

/// The drift check, given the identity of the plan the runner applied when
/// the host started the session (`None` when it applied none) and the
/// identity of the request's plan, both computed the way admission computes
/// a plan's identity for `plan_drift` (CONTRACT §10.1). A mismatch is the
/// `plan_drift` refusal with this detail: identities only, never plan
/// content.
pub fn check_drift(applied: Option<&str>, sent: &str) -> Result<(), PlanDriftDetail> {
    if applied == Some(sent) {
        Ok(())
    } else {
        Err(PlanDriftDetail::new(applied.map(str::to_owned), sent))
    }
}

/// The `session.adopt` reply. Decoded leniently.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`AdoptReply::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AdoptReply {
    /// Always `true` on a reply: the caller is the session's owner.
    pub owner_bound: bool,
    /// The identity of the plan the session is bound to, which is also the
    /// request plan's identity.
    pub applied_plan_digest: String,
    /// The session's last committed ordinal when it was adopted, so the
    /// caller knows where the transcript stands. Absent when the session has
    /// no message yet, as in `session.head`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_ordinal: Option<u64>,
}

impl AdoptReply {
    /// A reply binding the owner to the plan whose digest is
    /// `applied_plan_digest`, on a session with no message yet.
    pub fn new(applied_plan_digest: impl Into<String>) -> Self {
        Self {
            owner_bound: true,
            applied_plan_digest: applied_plan_digest.into(),
            last_ordinal: None,
        }
    }

    pub fn with_last_ordinal(mut self, last_ordinal: u64) -> Self {
        self.last_ordinal = Some(last_ordinal);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};
    use serde_json::json;

    /// Applies the module documentation's check order (caller, then
    /// `send_id`, then plan drift) to one vector case. The wire crate has no runner, so this is the reference
    /// reading of the vectors, not a runner implementation: the session's
    /// state and the plan identities come from the case.
    fn decide(case: &Value) -> Result<AdoptReply, (String, Option<Value>)> {
        let caller = case["caller"].as_str().unwrap();
        let owner = case["session"]["owner"].as_str();
        if caller == "direct" || owner.is_some_and(|owner| owner != caller) {
            return Err((errors::SCOPE_OWNER_MISMATCH.into(), None));
        }
        let request: AdoptRequest = serde_json::from_value(case["request"].clone()).unwrap();
        for earlier in case["earlier"].as_array().unwrap() {
            let earlier_request: AdoptRequest =
                serde_json::from_value(earlier["request"].clone()).unwrap();
            if earlier_request.send_id != request.send_id {
                continue;
            }
            return match request.reused_field(&earlier_request) {
                Some(field) => Err((
                    errors::SEND_ID_REUSE.into(),
                    Some(json!({ "field": field })),
                )),
                None => Ok(serde_json::from_value(earlier["reply"].clone()).unwrap()),
            };
        }
        let applied = case["session"]["applied_plan_digest"].as_str();
        let sent = case["sent_plan_digest"].as_str().unwrap();
        if let Err(detail) = check_drift(applied, sent) {
            return Err((
                errors::PLAN_DRIFT.into(),
                Some(serde_json::to_value(detail).unwrap()),
            ));
        }
        let reply = AdoptReply::new(sent);
        Ok(match case["session"]["last_ordinal"].as_u64() {
            Some(ordinal) => reply.with_last_ordinal(ordinal),
            None => reply,
        })
    }

    #[test]
    fn adopt_requests_and_replies_round_trip_and_refuse_by_field() {
        let file = vectors::load("adopt.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<AdoptRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error =
                serde_json::from_value::<AdoptRequest>(case["request"].clone()).expect_err(name);
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        for case in vectors::cases(&file, "replies") {
            let reply: AdoptReply =
                vectors::round_trip(case["name"].as_str().unwrap(), &case["reply"]);
            assert!(reply.owner_bound);
        }
        let lenient: AdoptReply = serde_json::from_value(
            json!({"owner_bound": true, "applied_plan_digest": "d", "future": 1}),
        )
        .unwrap();
        assert_eq!(lenient, AdoptReply::new("d"));
    }

    #[test]
    fn adopt_cases_follow_the_pinned_order_and_never_touch_the_transcript() {
        let file = vectors::load("adopt.json");
        let mut seen = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "cases") {
            let name = case["name"].as_str().unwrap();
            for counter in ["messages_written", "runs_started", "runs_interrupted"] {
                assert_eq!(case[counter], 0, "{name}: {counter}");
            }
            vectors::round_trip::<AdoptRequest>(name, &case["request"]);
            match (decide(case), case.get("refusal")) {
                (Ok(reply), None) => {
                    let expected: AdoptReply = vectors::round_trip(name, &case["reply"]);
                    assert_eq!(reply, expected, "{name}");
                    assert!(reply.owner_bound, "{name}");
                    let replayed = case["earlier"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .any(|earlier| earlier["request"] == case["request"]);
                    let writes = if replayed { "nothing" } else { "adoption" };
                    assert_eq!(case["writes"], writes, "{name}");
                    seen.insert(if replayed { "replay" } else { "bound" });
                }
                (Err((code, detail)), Some(refusal)) => {
                    assert_eq!(refusal["code"], code, "{name}");
                    assert_eq!(refusal.get("detail"), detail.as_ref(), "{name}");
                    assert!(errors::CODES.contains(&code.as_str()), "{name}");
                    assert!(!errors::is_retryable(&code), "{name}");
                    assert_eq!(case["writes"], "nothing", "{name}");
                    if code == errors::PLAN_DRIFT {
                        let detail: errors::PlanDriftDetail =
                            vectors::round_trip(name, &refusal["detail"]);
                        assert_eq!(
                            detail.frozen.as_deref(),
                            case["session"]["applied_plan_digest"].as_str(),
                            "{name}"
                        );
                    }
                    seen.insert(match code.as_str() {
                        errors::SEND_ID_REUSE => errors::SEND_ID_REUSE,
                        errors::PLAN_DRIFT => errors::PLAN_DRIFT,
                        errors::SCOPE_OWNER_MISMATCH => errors::SCOPE_OWNER_MISMATCH,
                        other => panic!("{name}: unexpected code {other}"),
                    });
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
        for kind in [
            "bound",
            "replay",
            errors::SEND_ID_REUSE,
            errors::PLAN_DRIFT,
            errors::SCOPE_OWNER_MISMATCH,
        ] {
            assert!(seen.contains(kind), "no vector for {kind}");
        }
        let active = vectors::cases(&file, "cases")
            .iter()
            .find(|case| !case["session"]["active_run"].is_null())
            .expect("a vector adopts during an active run");
        assert!(active.get("reply").is_some());
    }

    #[test]
    fn reuse_names_plan_and_drift_carries_identities_only() {
        let plan = |tools: Value| {
            let mut plan = Map::new();
            plan.insert("tools".into(), tools);
            plan
        };
        let first = AdoptRequest::new("s", plan(json!(["read"])));
        assert_eq!(first.reused_field(&first.clone()), None);
        assert_eq!(
            AdoptRequest::new("s", plan(json!(["write"]))).reused_field(&first),
            Some(PLAN_FIELD)
        );
        assert_eq!(check_drift(Some("a"), "a"), Ok(()));
        assert_eq!(
            check_drift(Some("a"), "b"),
            Err(errors::PlanDriftDetail::new(Some("a".into()), "b"))
        );
        assert_eq!(
            serde_json::to_value(check_drift(None, "b").unwrap_err()).unwrap(),
            json!({"sent": "b"})
        );
    }
}
