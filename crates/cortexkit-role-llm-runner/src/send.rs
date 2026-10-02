//! `session.send` from the session's owner: steer, queue and interrupt.
//!
//! A runner that declares `steer` or `queue` accepts an owner's prompt with
//! the matching `delivery` mode. One idempotency key, `send_id`, covers
//! every mode, and the mode is part of the send's identity:
//!
//! - the same `send_id` with the same payload and mode is answered with the
//!   existing state, so a retry looks like success;
//! - the same `send_id` with a different payload or mode is refused
//!   `send_id_reuse`, naming the field that differs.
//!
//! The runner replies only after the record
//! holding the message and its `send_id` (and, for a steered or queued
//! prompt, its PreUser output) is durable.
//!
//! The first send of a session admits it; its reply carries the admission
//! facts ([`SendReply::baseline`]).

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::baseline::Baseline;

/// Steer delivery receipt states.
pub mod delivered_as {
    /// Delivered: added to the running turn at a step boundary.
    pub const STEP: &str = "step";
    /// Delivered: started or carried by a turn (`ref` is the `run_id`).
    pub const TURN: &str = "turn";
    /// Accepted, not yet delivered, still deliverable.
    pub const PENDING: &str = "pending";
    /// Accepted, but delivery cannot be confirmed; the owner records
    /// `outcome_unknown` and never re-sends.
    pub const UNKNOWN: &str = "unknown";
}

/// A steer delivery state, classified.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeliveredAs {
    Step,
    Turn,
    Pending,
    Unknown,
    Other(String),
}

impl DeliveredAs {
    pub fn parse(s: &str) -> Self {
        match s {
            delivered_as::STEP => Self::Step,
            delivered_as::TURN => Self::Turn,
            delivered_as::PENDING => Self::Pending,
            delivered_as::UNKNOWN => Self::Unknown,
            other => Self::Other(other.to_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Step => delivered_as::STEP,
            Self::Turn => delivered_as::TURN,
            Self::Pending => delivered_as::PENDING,
            Self::Unknown => delivered_as::UNKNOWN,
            Self::Other(s) => s.as_str(),
        }
    }
}

/// A steer delivery receipt.
///
/// An unknown `as` value decodes as itself rather than failing.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Delivered {
    /// One of [`delivered_as`]; any other value decodes as a plain string.
    #[serde(rename = "as")]
    pub r#as: String,
    /// An opaque string: a stored row id, or the run id when `as` is `turn`.
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
}

/// Type alias for [`Delivered`].
pub type SteerReceipt = Delivered;
/// Type alias for [`Delivered`].
pub type SteerDelivery = Delivered;

impl Delivered {
    pub fn new(r#as: impl Into<String>) -> Self {
        Self {
            r#as: r#as.into(),
            r#ref: None,
        }
    }

    pub fn with_ref(mut self, r#ref: impl Into<String>) -> Self {
        self.r#ref = Some(r#ref.into());
        self
    }

    pub fn r#as(&self) -> DeliveredAs {
        DeliveredAs::parse(&self.r#as)
    }

    /// Whether the steer was delivered (`step` or `turn`).
    pub fn is_delivered(&self) -> bool {
        matches!(self.r#as(), DeliveredAs::Step | DeliveredAs::Turn)
    }
}

/// The `field` a refusal names for an unknown delivery mode, or for a
/// `send_id` reused with another mode.
pub const DELIVERY_FIELD: &str = "delivery";

/// How a prompt is delivered when a run is already active.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Delivery {
    /// The next turn after the running one. The default.
    Queue,
    /// Added to the running turn at its next step boundary, after any
    /// pending tool result, never between a call and its result. With no run
    /// active, a plain send.
    Steer,
    /// Written durably first; then the running turn is cancelled (it ends
    /// `cancelled`) and the message starts. A running tool call is waited on
    /// up to its deadline, never killed.
    Interrupt,
}

impl Delivery {
    /// The mode's string in a request's `delivery`, which is also the name of
    /// the capability group a runner declares to accept it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queue => crate::capabilities::QUEUE,
            Self::Steer => crate::capabilities::STEER,
            Self::Interrupt => crate::capabilities::INTERRUPT,
        }
    }
}

/// The `detail` of a `delivery_unsupported` refusal for `delivery`.
pub fn delivery_unsupported_detail(delivery: Delivery) -> Value {
    serde_json::json!({ DELIVERY_FIELD: delivery.as_str() })
}

fn deserialize_delivery<'de, D>(deserializer: D) -> Result<Option<Delivery>, D::Error>
where
    D: Deserializer<'de>,
{
    crate::strict::optional(
        deserializer,
        DELIVERY_FIELD,
        "\"queue\", \"steer\" or \"interrupt\"",
    )
}

/// The role's fields of a `session.send` request.
///
/// Lenient on unknown fields: a runner's own send parameters (model,
/// generation settings and the like) ride beside these and are kept in
/// [`SendRequest::runner_params`]. The `delivery` VALUE is strict: an
/// unknown mode is refused naming `delivery`, never read as `queue`.
///
/// Non-exhaustive so later optional members are additive: build one with
/// [`SendRequest::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SendRequest {
    /// The prompt text.
    pub prompt: String,
    /// The idempotency key. The owner derives it from what it delivers,
    /// never from the attempt.
    pub send_id: String,
    /// Absent means [`Delivery::Queue`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_delivery"
    )]
    pub delivery: Option<Delivery>,
    /// The sender's mark: the prompt is model-visible but not a human turn.
    /// The runner records it and does not act on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mark: Option<Value>,
    /// The session's fetch plan, carried verbatim when the runner declares
    /// `plans`. The first send establishes the session's plan and freezes its
    /// fetched manifest. An identical later repeat does not fetch again;
    /// a different plan is refused as `plan_drift` so an ordinary send cannot
    /// replace the session's frozen manifest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<Map<String, Value>>,
    /// The runner's own send parameters, kept as received.
    #[serde(flatten)]
    pub runner_params: Map<String, Value>,
}

impl SendRequest {
    pub fn new(prompt: impl Into<String>, send_id: impl Into<String>) -> Self {
        Self {
            prompt: prompt.into(),
            send_id: send_id.into(),
            delivery: None,
            mark: None,
            plan: None,
            runner_params: Map::new(),
        }
    }

    pub fn with_delivery(mut self, delivery: Delivery) -> Self {
        self.delivery = Some(delivery);
        self
    }

    pub fn with_mark(mut self, mark: Value) -> Self {
        self.mark = Some(mark);
        self
    }

    pub fn with_plan(mut self, plan: Map<String, Value>) -> Self {
        self.plan = Some(plan);
        self
    }

    pub fn with_runner_params(mut self, runner_params: Map<String, Value>) -> Self {
        self.runner_params = runner_params;
        self
    }

    /// The delivery mode, with absence read as [`Delivery::Queue`].
    pub fn delivery(&self) -> Delivery {
        self.delivery.unwrap_or(Delivery::Queue)
    }

    /// The request's delivery mode if `declared` (a runner's module-level
    /// capabilities) includes its group, or the mode to refuse as
    /// `delivery_unsupported` ([`delivery_unsupported_detail`]). An
    /// undeclared mode is refused, never delivered as another mode.
    pub fn check_delivery<S: AsRef<str>>(&self, declared: &[S]) -> Result<Delivery, Delivery> {
        let delivery = self.delivery();
        if declared
            .iter()
            .any(|name| name.as_ref() == delivery.as_str())
        {
            Ok(delivery)
        } else {
            Err(delivery)
        }
    }

    /// Refuse a plan-bearing send with `invalid_params {field: "plan"}`
    /// unless the runner's module-level capabilities declare `plans`.
    /// A request without a plan passes, regardless of the declaration.
    pub fn check_plan<S: AsRef<str>>(&self, declared: &[S]) -> Result<(), &'static str> {
        if self.plan.is_some()
            && !declared
                .iter()
                .any(|name| name.as_ref() == crate::capabilities::PLANS)
        {
            Err("plan")
        } else {
            Ok(())
        }
    }

    /// Admission's check of the request's `plan` against `declared` (a
    /// runner's module-level capabilities): `Err` with the `field` of an
    /// `invalid_params` refusal (`plan.compaction_item`) when the plan names
    /// a compaction provider and the runner does not declare `compaction`.
    /// A request without a plan passes.
    pub fn check_compaction_item<S: AsRef<str>>(&self, declared: &[S]) -> Result<(), &'static str> {
        match &self.plan {
            Some(plan) => crate::compaction::check_plan_compaction(plan, declared),
            None => Ok(()),
        }
    }
}

/// Send reply states. A decoder keeps any other value as a plain string
/// rather than failing.
pub mod states {
    /// The message is in a running turn: `run_id` names the run that
    /// carries it.
    pub const ACTIVE: &str = "active";
    /// The run that carried the message has ended, with `reason`.
    pub const FINISHED: &str = "finished";
    /// The message is durably queued as `submission_id` and will start a new
    /// run of its own.
    pub const PENDING: &str = "pending";
}

/// The `session.send` reply. Decoded leniently.
///
/// Non-exhaustive so later optional members are additive: use
/// [`SendReply::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SendReply {
    /// One of [`states`]; any other value decodes as a plain string.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// On the reply to the send that admitted the session: what was frozen,
    /// read from the record the runner wrote. A retry of that send with the
    /// same `send_id` gets the same facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Baseline>,
    /// Steer delivery receipt, when applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered: Option<Delivered>,
}

impl SendReply {
    pub fn new(state: impl Into<String>) -> Self {
        Self {
            state: state.into(),
            run_id: None,
            submission_id: None,
            reason: None,
            baseline: None,
            delivered: None,
        }
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = Some(run_id.into());
        self
    }

    pub fn with_submission_id(mut self, submission_id: impl Into<String>) -> Self {
        self.submission_id = Some(submission_id.into());
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_baseline(mut self, baseline: Baseline) -> Self {
        self.baseline = Some(baseline);
        self
    }

    pub fn with_delivered(mut self, delivered: Delivered) -> Self {
        self.delivered = Some(delivered);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn send_vectors_decode_and_round_trip() {
        let file = vectors::load("send.json");
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &case["request"]);
            let delivery = serde_json::to_value(request.delivery()).unwrap();
            assert_eq!(delivery, case["delivery"], "{name}");
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error =
                serde_json::from_value::<SendRequest>(case["request"].clone()).expect_err(name);
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        for case in vectors::cases(&file, "replies") {
            let name = case["name"].as_str().unwrap();
            let reply: SendReply = vectors::round_trip(name, &case["reply"]);
            assert_eq!(
                reply.baseline.is_some(),
                case["admitted"].as_bool().unwrap(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let code = case["refusal"]["code"].as_str().unwrap();
            assert_eq!(
                errors::is_retryable(code),
                case["retryable"].as_bool().unwrap(),
                "{name}"
            );
            if let Some(field) = case["field"].as_str() {
                assert_eq!(
                    errors::refused_field(code, case["refusal"].get("detail")),
                    Some(field),
                    "{name}"
                );
            }
        }
    }

    #[test]
    fn undeclared_delivery_modes_are_refused_by_name() {
        let file = vectors::load("send.json");
        for case in vectors::cases(&file, "delivery_checks") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &case["request"]);
            let declared: Vec<&str> = case["declares"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_str().unwrap())
                .collect();
            match (request.check_delivery(&declared), case.get("refusal")) {
                (Ok(_), None) => {}
                (Err(mode), Some(refusal)) => {
                    assert_eq!(refusal["code"], errors::DELIVERY_UNSUPPORTED, "{name}");
                    assert_eq!(
                        delivery_unsupported_detail(mode),
                        refusal["detail"],
                        "{name}"
                    );
                    assert!(!errors::is_retryable(errors::DELIVERY_UNSUPPORTED));
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
    }

    #[test]
    fn a_compaction_item_is_refused_at_admission_without_the_compaction_group() {
        let file = vectors::load("send.json");
        for case in vectors::cases(&file, "compaction_item_checks") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &case["request"]);
            let declared: Vec<&str> = case["declares"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_str().unwrap())
                .collect();
            match (
                request.check_compaction_item(&declared),
                case.get("refusal"),
            ) {
                (Ok(()), None) => {}
                (Err(field), Some(refusal)) => {
                    assert_eq!(refusal["code"], errors::INVALID_PARAMS, "{name}");
                    assert_eq!(
                        errors::refused_field(errors::INVALID_PARAMS, refusal.get("detail")),
                        Some(field),
                        "{name}"
                    );
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
    }

    #[test]
    fn a_plan_is_refused_without_the_plans_group() {
        let file = vectors::load("plans.json");
        for case in vectors::cases(&file, "admission_checks") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &case["request"]);
            if let Some(reply) = case.get("reply") {
                vectors::round_trip::<SendReply>(name, reply);
            }
            let declared: Vec<&str> = case["declares"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c.as_str().unwrap())
                .collect();
            match (request.check_plan(&declared), case.get("refusal")) {
                (Ok(()), None) => {}
                (Err(field), Some(refusal)) => {
                    assert_eq!(refusal["code"], errors::INVALID_PARAMS, "{name}");
                    assert_eq!(
                        errors::refused_field(errors::INVALID_PARAMS, refusal.get("detail")),
                        Some(field),
                        "{name}"
                    );
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
    }

    #[test]
    fn frozen_plan_vectors_round_trip() {
        // Round-trip first/later send requests, baseline replies and drift
        // details. This wire crate has no runner fetch/freeze implementation,
        // so these vectors do not test comparing plans across sends.
        let file = vectors::load("plans.json");
        for case in vectors::cases(&file, "frozen_cases") {
            let name = case["name"].as_str().unwrap();
            vectors::round_trip::<SendRequest>(name, &case["first_send"]);
            vectors::round_trip::<SendRequest>(name, &case["later_send"]);
            vectors::round_trip::<crate::baseline::BaselineReply>(name, &case["baseline"]);
            if let Some(refusal) = case.get("refusal") {
                assert_eq!(refusal["code"], errors::PLAN_DRIFT, "{name}");
                vectors::round_trip::<errors::PlanDriftDetail>(name, &refusal["detail"]);
            }
        }
    }

    #[test]
    fn runner_params_ride_beside_the_role_fields() {
        let raw = serde_json::json!({
            "prompt": "hi",
            "send_id": "s-1",
            "delivery": "steer",
            "model": {"id": "m"}
        });
        let request: SendRequest = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(request.delivery(), Delivery::Steer);
        assert_eq!(request.runner_params["model"]["id"], "m");
        assert_eq!(serde_json::to_value(&request).unwrap(), raw);
        assert_eq!(SendRequest::new("hi", "s-2").delivery(), Delivery::Queue);
    }

    #[test]
    fn steer_delivery_receipt_vectors_decode_and_round_trip() {
        let file = vectors::load("send.json");
        let mut tested_as = std::collections::BTreeSet::new();
        let mut tested_with_ref = 0;
        let mut tested_without_ref = 0;
        for case in vectors::cases(&file, "replies") {
            let name = case["name"].as_str().unwrap();
            let reply: SendReply = vectors::round_trip(name, &case["reply"]);
            if let Some(delivered) = &reply.delivered {
                tested_as.insert(delivered.r#as.clone());
                if delivered.r#ref.is_some() {
                    tested_with_ref += 1;
                } else {
                    tested_without_ref += 1;
                }
            }
        }
        assert!(tested_as.contains(delivered_as::STEP));
        assert!(tested_as.contains(delivered_as::TURN));
        assert!(tested_as.contains(delivered_as::PENDING));
        assert!(tested_as.contains(delivered_as::UNKNOWN));
        assert!(tested_as.contains("future_delivery"));
        assert_eq!(tested_with_ref, 5);
        assert_eq!(tested_without_ref, 4);
    }

    #[test]
    fn open_decoding_of_unknown_as_preserves_value() {
        let raw = serde_json::json!({
            "state": "active",
            "delivered": {
                "as": "custom_extension",
                "ref": "ext-99"
            }
        });
        let reply: SendReply = serde_json::from_value(raw.clone()).unwrap();
        let delivered = reply.delivered.as_ref().unwrap();
        assert_eq!(delivered.r#as, "custom_extension");
        assert_eq!(delivered.r#ref.as_deref(), Some("ext-99"));
        assert_eq!(
            delivered.r#as(),
            DeliveredAs::Other("custom_extension".into())
        );
        assert!(!delivered.is_delivered());
        let encoded = serde_json::to_value(&reply).unwrap();
        assert_eq!(encoded, raw);
    }
}
