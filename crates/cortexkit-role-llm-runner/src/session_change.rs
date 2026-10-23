//! The `session_change` group: an owner's mid-session changes.
//!
//! `session.refresh` fetches a new plan's items, under one deadline exactly
//! as admission does, and holds them as the session's pending change, under
//! a generation the owner chooses and a policy naming the rung
//! ([`crate::rungs`]) at which it applies. `session.refresh_policy` changes
//! the pending change's rung, and `session.flush_prefix` applies any pending
//! change, and drops the frozen prefix, at the start of the next run.
//!
//! A change never applies inside a run that is already running: the
//! earliest it applies is the start of the next run, and the `immediately`
//! rung means "from the next run". No step ever sees text or tools its run
//! did not start with.
//!
//! After a lost reply, the owner reads `session.baseline`: its
//! `accepted_generation`, `applied_generation`, `pending` and `effective`
//! ([`crate::baseline::Baseline`]) carry everything these replies do.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{baseline::PendingChange, errors::PlanDriftDetail, rungs};

/// The surfaces a change policy applies to, as a baseline's `rungs` names
/// them and a `rung_unsupported` refusal reports them.
pub mod surfaces {
    /// The model-facing tool set.
    pub const TOOLS: &str = "tools";
    /// The joined system text.
    pub const SYSTEM_TEXT: &str = "system_text";
}

/// The `session.refresh` request. Strict: an unknown field is refused
/// `invalid_params` naming it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {
    /// The full fetch plan, as [`crate::send::SendRequest::plan`] carries
    /// it, passed to providers verbatim.
    pub plan: Map<String, Value>,
    /// One rung name from [`rungs::ALL`].
    pub policy: String,
    /// Chosen by the owner, strictly increasing per session.
    pub generation: u64,
}

impl RefreshRequest {
    /// Check output-reference plan members before fetching or storing a change.
    pub fn check_output_refs(&self, advertised: bool) -> Result<(), &'static str> {
        crate::output_refs::check_plan(&Value::Object(self.plan.clone()), advertised)
    }
    pub fn new(plan: Map<String, Value>, policy: impl Into<String>, generation: u64) -> Self {
        Self {
            plan,
            policy: policy.into(),
            generation,
        }
    }

    /// Refuse a policy that names no rung of [`rungs::ALL`], with the field
    /// an `invalid_params` refusal names. A known rung the session does not
    /// support is `rung_unsupported` instead, which only the runner can tell.
    pub fn check_policy(&self) -> Result<(), &'static str> {
        check_policy(&self.policy)
    }
}

/// The `session.refresh` answer. Decoded leniently.
///
/// An identical repeat (the same generation, plan identity and policy) gets
/// the same answer as the request it repeats.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RefreshReply {
    /// The generation the runner accepted: the request's.
    pub accepted_generation: u64,
    /// The pending change, now this generation with this policy.
    pub pending: PendingChange,
    /// Whether a run is running now, so the next run is queued behind it.
    pub run_in_progress: bool,
    /// Whether the next run uses the new plan: the `immediately` rung, or a
    /// rung whose point the session already meets. When false, the change
    /// waits for a later rebuild, and the current plan stays live until
    /// `session.baseline` shows `applied_generation` at or above it.
    pub applies_from_next_run: bool,
}

impl RefreshReply {
    pub fn new(pending: PendingChange, run_in_progress: bool, applies_from_next_run: bool) -> Self {
        Self {
            accepted_generation: pending.generation,
            pending,
            run_in_progress,
            applies_from_next_run,
        }
    }
}

/// The `session.refresh_policy` request: a new rung for the pending change.
/// Strict. Idempotent on `send_id`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshPolicyRequest {
    /// The generation the caller believes is pending.
    pub generation: u64,
    /// One rung name from [`rungs::ALL`].
    pub policy: String,
    pub send_id: String,
}

impl RefreshPolicyRequest {
    pub fn new(generation: u64, policy: impl Into<String>, send_id: impl Into<String>) -> Self {
        Self {
            generation,
            policy: policy.into(),
            send_id: send_id.into(),
        }
    }

    /// As [`RefreshRequest::check_policy`].
    pub fn check_policy(&self) -> Result<(), &'static str> {
        check_policy(&self.policy)
    }

    /// The first field in which this request differs from `earlier`, sent
    /// under the same `send_id`: the field a `send_id_reuse` refusal names.
    /// `None` for an identical repeat, which gets the earlier answer.
    pub fn reused_field(&self, earlier: &Self) -> Option<&'static str> {
        if self.generation != earlier.generation {
            Some("generation")
        } else if self.policy != earlier.policy {
            Some("policy")
        } else {
            None
        }
    }
}

/// The `session.refresh_policy` answer. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RefreshPolicyReply {
    pub pending: PendingChange,
}

impl RefreshPolicyReply {
    pub fn new(pending: PendingChange) -> Self {
        Self { pending }
    }
}

/// The `session.flush_prefix` request. Strict. Idempotent on `send_id`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FlushPrefixRequest {
    pub send_id: String,
}

impl FlushPrefixRequest {
    pub fn new(send_id: impl Into<String>) -> Self {
        Self {
            send_id: send_id.into(),
        }
    }
}

/// The `session.flush_prefix` answer. Decoded leniently.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct FlushPrefixReply {
    /// The pending change the flush applies at the start of the next run;
    /// absent when none was pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_generation: Option<u64>,
}

impl FlushPrefixReply {
    pub fn new(pending_generation: Option<u64>) -> Self {
        Self { pending_generation }
    }
}

fn check_policy(policy: &str) -> Result<(), &'static str> {
    if rungs::ALL.contains(&policy) {
        Ok(())
    } else {
        Err("policy")
    }
}

/// The detail of a `rung_unsupported` refusal: the surface and the rung the
/// session does not support on it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RungUnsupportedDetail {
    /// One of [`surfaces`].
    pub surface: String,
    pub rung: String,
}

impl RungUnsupportedDetail {
    pub fn new(surface: impl Into<String>, rung: impl Into<String>) -> Self {
        Self {
            surface: surface.into(),
            rung: rung.into(),
        }
    }
}

/// The detail of a `generation_stale` refusal: the highest generation the
/// runner has accepted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct GenerationStaleDetail {
    pub accepted: u64,
}

impl GenerationStaleDetail {
    pub fn new(accepted: u64) -> Self {
        Self { accepted }
    }
}

/// The detail of a `pending_superseded` refusal: the generation the request
/// named, and the one pending now.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct PendingSupersededDetail {
    pub generation: u64,
    pub pending: u64,
}

impl PendingSupersededDetail {
    pub fn new(generation: u64, pending: u64) -> Self {
        Self {
            generation,
            pending,
        }
    }
}

/// The detail of an `already_applied` refusal: the generation the request
/// named, and how it was applied ([`crate::baseline::applied_by`]).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct AlreadyAppliedDetail {
    pub generation: u64,
    pub applied_by: String,
}

impl AlreadyAppliedDetail {
    pub fn new(generation: u64, applied_by: impl Into<String>) -> Self {
        Self {
            generation,
            applied_by: applied_by.into(),
        }
    }
}

/// The detail of a `tool_outside_host_set` refusal: the model-facing names
/// of the plan's tools that are not in the host's fixed set, sorted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolOutsideHostSetDetail {
    pub names: Vec<String>,
}

impl ToolOutsideHostSetDetail {
    pub fn new(names: Vec<String>) -> Self {
        Self { names }
    }
}

/// A tool's definition as a host registers it: what decides whether a
/// plan's tool is inside a fixed host tool set.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolDefinition {
    /// The model-facing name.
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl ToolDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
        }
    }
}

/// The names of the `offered` tools outside the `host` set, sorted and each
/// once: a tool is inside only when the host registered a definition
/// identical in name, description and input schema. A kept name with a
/// changed description or schema is outside. Empty when every tool is
/// inside.
pub fn outside_host_set(host: &[ToolDefinition], offered: &[ToolDefinition]) -> Vec<String> {
    let mut names: Vec<String> = offered
        .iter()
        .filter(|tool| !host.contains(tool))
        .map(|tool| tool.name.clone())
        .collect();
    names.sort();
    names.dedup();
    names
}

/// How a `session.refresh` stands against the generations the runner has
/// accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshOrder {
    /// A generation above every accepted one: a new change.
    New,
    /// An identical repeat of an accepted refresh: answer as before.
    Repeat,
    /// Any other generation at or below the accepted one: refused
    /// `generation_stale`.
    Stale,
}

/// Classify a refresh of `generation`, with `plan_identity` and `policy`.
/// `accepted` is the highest accepted generation (`0` before any) and
/// `earlier` the plan identity and policy accepted under `generation`, if
/// it was.
pub fn refresh_order(
    accepted: u64,
    generation: u64,
    plan_identity: &str,
    policy: &str,
    earlier: Option<(&str, &str)>,
) -> RefreshOrder {
    if generation > accepted {
        RefreshOrder::New
    } else if earlier == Some((plan_identity, policy)) {
        RefreshOrder::Repeat
    } else {
        RefreshOrder::Stale
    }
}

/// Check an owner send's plan while a change may be pending: a plan that is
/// the effective one or the latest accepted one is not drift. Anything else
/// is `plan_drift`, whose `frozen` names the latest accepted plan (the
/// effective one when nothing was accepted since).
pub fn check_send_drift(
    effective: Option<&str>,
    accepted: Option<&str>,
    sent: &str,
) -> Result<(), PlanDriftDetail> {
    if effective == Some(sent) || accepted == Some(sent) {
        Ok(())
    } else {
        Err(PlanDriftDetail::new(
            accepted.or(effective).map(str::to_owned),
            sent,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{baseline::BaselineReply, capabilities, errors, ops, vectors};

    #[test]
    fn requests_round_trip_and_refuse_unknown_fields() {
        let file = vectors::load("session-change.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let op = case["op"].as_str().unwrap();
            let request = &case["request"];
            match op {
                ops::SESSION_REFRESH => {
                    let decoded: RefreshRequest = vectors::round_trip(name, request);
                    assert_eq!(decoded.check_policy(), Ok(()), "{name}");
                }
                ops::SESSION_REFRESH_POLICY => {
                    let decoded: RefreshPolicyRequest = vectors::round_trip(name, request);
                    assert_eq!(decoded.check_policy(), Ok(()), "{name}");
                }
                ops::SESSION_FLUSH_PREFIX => {
                    vectors::round_trip::<FlushPrefixRequest>(name, request);
                }
                other => panic!("{name}: {other} is not a session_change op"),
            }
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let request = case["request"].clone();
            let field = case["field"].as_str().unwrap();
            let refused = match case["op"].as_str().unwrap() {
                ops::SESSION_REFRESH => match serde_json::from_value::<RefreshRequest>(request) {
                    Ok(decoded) => decoded.check_policy() == Err(field),
                    Err(error) => error.to_string().contains(field),
                },
                ops::SESSION_REFRESH_POLICY => {
                    match serde_json::from_value::<RefreshPolicyRequest>(request) {
                        Ok(decoded) => decoded.check_policy() == Err(field),
                        Err(error) => error.to_string().contains(field),
                    }
                }
                ops::SESSION_FLUSH_PREFIX => serde_json::from_value::<FlushPrefixRequest>(request)
                    .is_err_and(|error| error.to_string().contains(field)),
                other => panic!("{name}: {other}"),
            };
            assert!(refused, "{name}: not refused naming {field}");
        }
    }

    #[test]
    fn answers_round_trip_and_keep_their_rules() {
        let file = vectors::load("session-change.json");
        let mut shapes = Vec::new();
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let answer = &case["answer"];
            match case["op"].as_str().unwrap() {
                ops::SESSION_REFRESH => {
                    let reply: RefreshReply = vectors::round_trip(name, answer);
                    assert_eq!(
                        reply.accepted_generation, reply.pending.generation,
                        "{name}"
                    );
                    assert!(
                        rungs::ALL.contains(&reply.pending.policy.as_str()),
                        "{name}"
                    );
                    shapes.push((reply.run_in_progress, reply.applies_from_next_run));
                }
                ops::SESSION_REFRESH_POLICY => {
                    vectors::round_trip::<RefreshPolicyReply>(name, answer);
                }
                ops::SESSION_FLUSH_PREFIX => {
                    vectors::round_trip::<FlushPrefixReply>(name, answer);
                }
                other => panic!("{name}: {other}"),
            }
        }
        for shape in [(false, true), (true, true), (false, false), (true, false)] {
            assert!(shapes.contains(&shape), "no refresh answer {shape:?}");
        }
        // After a lost reply, `session.baseline` carries everything a
        // refresh answer said.
        for case in vectors::cases(&file, "baselines_after_refresh") {
            let name = case["name"].as_str().unwrap();
            let reply: BaselineReply = vectors::round_trip(name, &case["answer"]);
            let baseline = reply.baseline.expect(name);
            let refreshed: RefreshReply = serde_json::from_value(case["refresh"].clone()).unwrap();
            assert_eq!(
                baseline.accepted_generation, refreshed.accepted_generation,
                "{name}"
            );
            match &baseline.pending {
                Some(pending) => assert_eq!(pending, &refreshed.pending, "{name}"),
                None => {
                    assert!(
                        baseline.applied_generation >= refreshed.accepted_generation,
                        "{name}"
                    );
                    assert_eq!(
                        baseline.effective.as_ref().map(|e| e.generation),
                        Some(baseline.applied_generation),
                        "{name}"
                    );
                }
            }
        }
    }

    #[test]
    fn refusals_carry_their_typed_detail() {
        let file = vectors::load("session-change.json");
        let group = capabilities::group_ops(capabilities::SESSION_CHANGE).unwrap();
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            assert!(group.contains(&case["op"].as_str().unwrap()), "{name}");
            let refusal = &case["refusal"];
            let detail = &refusal["detail"];
            match refusal["code"].as_str().unwrap() {
                errors::RUNG_UNSUPPORTED => {
                    let detail: RungUnsupportedDetail = vectors::round_trip(name, detail);
                    assert!(
                        [surfaces::TOOLS, surfaces::SYSTEM_TEXT].contains(&detail.surface.as_str()),
                        "{name}"
                    );
                }
                errors::GENERATION_STALE => {
                    vectors::round_trip::<GenerationStaleDetail>(name, detail);
                }
                errors::PENDING_SUPERSEDED => {
                    let detail: PendingSupersededDetail = vectors::round_trip(name, detail);
                    assert!(detail.pending > detail.generation, "{name}");
                }
                errors::ALREADY_APPLIED => {
                    vectors::round_trip::<AlreadyAppliedDetail>(name, detail);
                }
                errors::TOOL_OUTSIDE_HOST_SET => {
                    let detail: ToolOutsideHostSetDetail = vectors::round_trip(name, detail);
                    assert!(!detail.names.is_empty(), "{name}");
                }
                errors::PLAN_DRIFT => {
                    vectors::round_trip::<PlanDriftDetail>(name, detail);
                }
                errors::FETCH_UNAVAILABLE | errors::PLAN_STALE | errors::TOOL_NAME_COLLISION => {}
                errors::SEND_ID_REUSE => {
                    assert!(errors::refused_field(errors::SEND_ID_REUSE, Some(detail)).is_some());
                }
                other => panic!("{name}: {other} is not a session_change refusal"),
            }
        }
    }

    #[test]
    fn host_set_membership_is_by_whole_definition() {
        let file = vectors::load("session-change.json");
        for case in vectors::cases(&file, "host_set") {
            let name = case["name"].as_str().unwrap();
            let host: Vec<ToolDefinition> = serde_json::from_value(case["host"].clone()).unwrap();
            let offered: Vec<ToolDefinition> =
                serde_json::from_value(case["offered"].clone()).unwrap();
            let outside: Vec<String> = serde_json::from_value(case["outside"].clone()).unwrap();
            assert_eq!(outside_host_set(&host, &offered), outside, "{name}");
        }
    }

    #[test]
    fn generations_and_drift_classify_as_recorded() {
        let file = vectors::load("session-change.json");
        for case in vectors::cases(&file, "generations") {
            let name = case["name"].as_str().unwrap();
            let earlier = case["earlier"]
                .as_object()
                .map(|e| (e["plan"].as_str().unwrap(), e["policy"].as_str().unwrap()));
            let order = refresh_order(
                case["accepted"].as_u64().unwrap(),
                case["generation"].as_u64().unwrap(),
                case["plan"].as_str().unwrap(),
                case["policy"].as_str().unwrap(),
                earlier,
            );
            let expected = match case["order"].as_str().unwrap() {
                "new" => RefreshOrder::New,
                "repeat" => RefreshOrder::Repeat,
                "stale" => RefreshOrder::Stale,
                other => panic!("{name}: {other}"),
            };
            assert_eq!(order, expected, "{name}");
        }
        for case in vectors::cases(&file, "drift") {
            let name = case["name"].as_str().unwrap();
            let result = check_send_drift(
                case["effective"].as_str(),
                case["accepted"].as_str(),
                case["sent"].as_str().unwrap(),
            );
            match case.get("refused") {
                None => assert_eq!(result, Ok(()), "{name}"),
                Some(detail) => {
                    assert_eq!(
                        result,
                        Err(serde_json::from_value(detail.clone()).unwrap()),
                        "{name}"
                    );
                }
            }
        }
    }
}
