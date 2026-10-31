//! Plan-named read admission (`app_readers`, CONTRACT §9.19).
//!
//! Principals are strings, matched exactly against the route's attested opener,
//! just like [`crate::delivery_order::PlanSender::module`]. A present `readers`
//! list restricts the six session reads in CONTRACT §9.19 and `send.status`;
//! absence keeps that runner's current read admission.

use serde_json::{Map, Value};

use crate::{capabilities, ops, session_change};

pub const PLAN_READERS: &str = "readers";
pub const PLAN_READERS_FIELD: &str = "plan.readers";

/// Decode the optional list, refusing malformed entries and exact duplicates.
/// Unlike absence, `null` is malformed; an empty array is a read restriction.
pub fn plan_readers(plan: &Map<String, Value>) -> Result<Option<Vec<String>>, &'static str> {
    let Some(raw) = plan.get(PLAN_READERS) else {
        return Ok(None);
    };
    let readers: Vec<String> =
        serde_json::from_value(raw.clone()).map_err(|_| PLAN_READERS_FIELD)?;
    let unique: std::collections::BTreeSet<_> = readers.iter().collect();
    if unique.len() != readers.len() {
        return Err(PLAN_READERS_FIELD);
    }
    Ok(Some(readers))
}

/// Admission gate and shape check. Even an empty list requires `app_readers`.
/// An error means `invalid_params` naming `plan.readers`, with no writes.
pub fn check_plan<S: AsRef<str>>(
    plan: &Map<String, Value>,
    declared: &[S],
) -> Result<(), &'static str> {
    if plan.contains_key(PLAN_READERS)
        && !declared
            .iter()
            .any(|group| group.as_ref() == capabilities::APP_READERS)
    {
        return Err(PLAN_READERS_FIELD);
    }
    plan_readers(plan).map(|_| ())
}

/// Runner-owned facts that a plan cannot attest for itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReadAdmission {
    /// The runner's current admission decision when `readers` is absent.
    pub current: bool,
    /// A local operator on a direct connection whose development admission
    /// still stands. A runner not admitting local operators leaves this false.
    pub local_operator: bool,
    /// The `send.status` request names this caller's own send.
    pub own_send: bool,
}

impl ReadAdmission {
    pub fn new(current: bool) -> Self {
        Self {
            current,
            ..Self::default()
        }
    }

    pub fn with_local_operator(mut self, admitted: bool) -> Self {
        self.local_operator = admitted;
        self
    }

    pub fn with_own_send(mut self, own_send: bool) -> Self {
        self.own_send = own_send;
        self
    }
}

/// May `caller` read through `op` under this session's admission policy?
///
/// `caller` is the route's attested opener, never a request field. `effective`
/// is the applied plan and `pending` the latest durably accepted refresh plan
/// ([`session_change::admission_plan`]). Reader and sender lists come from the
/// latter immediately; the implicit compaction-provider grant follows the
/// effective plan until the provider actually switches at a run boundary.
///
/// Without `readers`, returns `facts.current` for read operations: the runner
/// keeps its current admission, neither opening nor closing it. With a list,
/// an unlisted non-implicit caller returns false (`scope_owner_mismatch`, before
/// any work). Listed readers gain no writes, and `send.status` remains limited
/// to the owner and named senders reading their own sends whenever a
/// `readers` list is present. Any other operation returns false: its admission
/// is decided by its own rule, not here.
pub fn admit(
    effective: &Map<String, Value>,
    pending: Option<&Map<String, Value>>,
    caller: &str,
    owner: &str,
    op: &str,
    facts: ReadAdmission,
) -> Result<bool, &'static str> {
    let read = matches!(
        op,
        ops::SESSION_HEAD
            | ops::SESSION_READ
            | ops::SESSION_HISTORY
            | ops::RUN_STATUS
            | ops::ROLE_DESCRIBE
            | ops::SESSION_SUBSCRIBE
    );
    if !read && op != ops::SEND_STATUS {
        return Ok(false);
    }
    let plan = session_change::admission_plan(effective, pending);
    let Some(readers) = plan_readers(plan)? else {
        return Ok(facts.current);
    };
    if caller == owner {
        return Ok(true);
    }
    if op == ops::SEND_STATUS {
        return Ok(facts.own_send
            && crate::delivery_order::plan_senders(plan)?
                .iter()
                .any(|sender| sender.module == caller));
    }
    let provider = effective
        .get("compaction_item")
        .and_then(|item| item.get("provider"))
        .and_then(Value::as_str);
    if provider == Some(caller) || facts.local_operator {
        return Ok(true);
    }
    Ok(readers.iter().any(|reader| reader == caller))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn vectors() -> Value {
        serde_json::from_str(include_str!(
            "../test-vectors/llm-runner-v1/app-readers/admission.json"
        ))
        .unwrap()
    }

    #[test]
    fn shape_and_group_gate_vectors() {
        for case in vectors()["shape"].as_array().unwrap() {
            let declared: Vec<String> = serde_json::from_value(case["declared"].clone()).unwrap();
            let result = check_plan(case["plan"].as_object().unwrap(), &declared);
            assert_eq!(
                result.is_ok(),
                case["valid"].as_bool().unwrap(),
                "{}",
                case["name"]
            );
            if result.is_err() {
                assert_eq!(result, Err(PLAN_READERS_FIELD));
            }
        }
    }

    #[test]
    fn read_admission_vectors() {
        let plan = json!({"readers":["reader-a"], "senders":[{"module":"sender-b"}],
            "compaction_item":{"provider":"active-provider"}});
        for case in vectors()["reads"].as_array().unwrap() {
            let facts = ReadAdmission::new(true)
                .with_own_send(case["own_send"] == true)
                .with_local_operator(case["local_operator"] == true);
            assert_eq!(
                admit(
                    plan.as_object().unwrap(),
                    None,
                    case["caller"].as_str().unwrap(),
                    "owner",
                    case["op"].as_str().unwrap(),
                    facts
                )
                .unwrap(),
                case["admitted"].as_bool().unwrap(),
                "{case}"
            );
        }
    }

    #[test]
    fn absent_and_empty_lists_have_different_admission() {
        let absent = Map::new();
        let empty = json!({"readers":[]});
        for current in [false, true] {
            let facts = ReadAdmission::new(current);
            assert_eq!(
                admit(&absent, None, "reader", "owner", ops::SESSION_READ, facts),
                Ok(current)
            );
            assert_eq!(
                admit(
                    empty.as_object().unwrap(),
                    None,
                    "reader",
                    "owner",
                    ops::SESSION_READ,
                    facts
                ),
                Ok(false)
            );
        }
    }

    #[test]
    fn accepted_lists_switch_now_but_the_active_provider_switches_with_model_input() {
        let old = json!({"readers":["old-reader"], "compaction_item":{"provider":"old-provider"}});
        let new = json!({"readers":["new-reader"], "compaction_item":{"provider":"new-provider"}});
        let old = old.as_object().unwrap();
        let new = new.as_object().unwrap();
        for (caller, expected) in [
            ("old-reader", false),
            ("new-reader", true),
            ("old-provider", true),
            ("new-provider", false),
        ] {
            assert_eq!(
                admit(
                    old,
                    Some(new),
                    caller,
                    "owner",
                    ops::SESSION_READ,
                    ReadAdmission::default()
                ),
                Ok(expected)
            );
        }
        assert_eq!(
            admit(
                new,
                None,
                "old-provider",
                "owner",
                ops::SESSION_READ,
                ReadAdmission::default()
            ),
            Ok(false)
        );
        assert_eq!(
            admit(
                new,
                None,
                "new-provider",
                "owner",
                ops::SESSION_READ,
                ReadAdmission::default()
            ),
            Ok(true)
        );
        let replacement = Map::new();
        assert_eq!(
            admit(
                old,
                Some(&replacement),
                "new-reader",
                "owner",
                ops::SESSION_READ,
                ReadAdmission::default()
            ),
            Ok(false)
        );
    }
}
