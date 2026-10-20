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

/// A steer delivery state, classified. Non-exhaustive: a state the role adds
/// later must not break a consumer's `match`, which keeps a catch-all arm.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
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

/// `delivery` on the wire: a legacy mode string, or the ordered list of
/// the `delivery_order` group.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
enum WireDelivery {
    Mode(Delivery),
    Order(Vec<crate::delivery_order::Mode>),
}

/// Decode `delivery` as a mode string or a mode list, reading JSON `null`
/// as absent. Anything else, an unknown mode, an empty list or a list
/// naming a mode twice is refused with a message naming `delivery`.
fn deserialize_delivery<'de, D>(deserializer: D) -> Result<Option<WireDelivery>, D::Error>
where
    D: Deserializer<'de>,
{
    let refused = |value: &Value| {
        serde::de::Error::custom(format!(
            "{DELIVERY_FIELD} must be \"queue\", \"steer\" or \"interrupt\", or a non-empty \
             list of distinct delivery_order modes; got {value}"
        ))
    };
    let value = match Option::<Value>::deserialize(deserializer)? {
        None | Some(Value::Null) => return Ok(None),
        Some(value) => value,
    };
    if value.is_array() {
        let modes: Vec<crate::delivery_order::Mode> =
            serde_json::from_value(value.clone()).map_err(|_| refused(&value))?;
        let mut distinct = modes.clone();
        distinct.sort();
        distinct.dedup();
        if modes.is_empty() || distinct.len() != modes.len() {
            return Err(refused(&value));
        }
        return Ok(Some(WireDelivery::Order(modes)));
    }
    serde_json::from_value(value.clone())
        .map(|mode| Some(WireDelivery::Mode(mode)))
        .map_err(|_| refused(&value))
}

/// The wire shape of a `session.send` request, which [`SendRequest`] is
/// decoded from and encoded to, so `delivery` takes either form in one
/// field.
#[derive(Clone, Deserialize, Serialize)]
struct WireSendRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    send_id: String,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_delivery"
    )]
    delivery: Option<WireDelivery>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<Map<String, Value>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_retention"
    )]
    retention: Option<u64>,
    #[serde(flatten)]
    runner_params: Map<String, Value>,
}

impl From<WireSendRequest> for SendRequest {
    fn from(wire: WireSendRequest) -> Self {
        let (delivery, delivery_order) = match wire.delivery {
            None => (None, None),
            Some(WireDelivery::Mode(mode)) => (Some(mode), None),
            Some(WireDelivery::Order(modes)) => (None, Some(modes)),
        };
        Self {
            prompt: wire.prompt,
            send_id: wire.send_id,
            delivery,
            delivery_order,
            plan: wire.plan,
            retention: wire.retention,
            runner_params: wire.runner_params,
        }
    }
}

impl From<SendRequest> for WireSendRequest {
    fn from(request: SendRequest) -> Self {
        let delivery = match (request.delivery_order, request.delivery) {
            (Some(modes), _) => Some(WireDelivery::Order(modes)),
            (None, mode) => mode.map(WireDelivery::Mode),
        };
        Self {
            prompt: request.prompt,
            send_id: request.send_id,
            delivery,
            plan: request.plan,
            retention: request.retention,
            runner_params: request.runner_params,
        }
    }
}

fn deserialize_retention<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    crate::strict::optional(deserializer, "retention", "u64 seconds")
}

/// Role budget fields. Existing runner-specific values are kept verbatim;
/// only runners advertising `input_token_cap` apply the closed-member check.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Budget {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_input_cap"
    )]
    pub max_input_tokens: Option<u64>,
    #[serde(flatten)]
    pub runner_params: Map<String, Value>,
}

fn deserialize_input_cap<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    let value: u64 = Deserialize::deserialize(deserializer).map_err(|_| {
        serde::de::Error::custom("budget.max_input_tokens must be a u64 of at least 1")
    })?;
    if value == 0 {
        return Err(serde::de::Error::custom(
            "budget.max_input_tokens must be at least 1",
        ));
    }
    Ok(Some(value))
}

impl Budget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_max_input_tokens(mut self, limit: u64) -> Self {
        self.max_input_tokens = Some(limit);
        self
    }

    /// Validate before writing a send. `frozen` is the existing session cap;
    /// `None` means unlimited, including sessions first sent without a cap.
    /// Returns the field for an `invalid_params` refusal. Non-advertisers
    /// retain their previous rules for other budget members.
    pub fn check(&self, advertised: bool, frozen: Option<u64>) -> Result<(), String> {
        if let Some(limit) = self.max_input_tokens {
            if !advertised || limit == 0 || frozen.is_some_and(|old| limit > old) {
                return Err("budget.max_input_tokens".into());
            }
        }
        if advertised {
            for member in self.runner_params.keys() {
                if !matches!(
                    member.as_str(),
                    "nudges" | "finalize_at_tool_calls" | "finalize_text"
                ) {
                    return Err(format!("budget.{member}"));
                }
            }
            if let Some(nudges) = self.runner_params.get("nudges").and_then(Value::as_array) {
                for (index, nudge) in nudges.iter().enumerate() {
                    if let Some(members) = nudge.as_object() {
                        for member in members.keys() {
                            if !matches!(member.as_str(), "at_tool_calls" | "text") {
                                return Err(format!("budget.nudges[{index}].{member}"));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
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
#[serde(from = "WireSendRequest", into = "WireSendRequest")]
#[non_exhaustive]
pub struct SendRequest {
    /// The prompt text: required, and not empty, on every send but a
    /// template send, which carries no `prompt` and whose text the runner
    /// renders from the plan's template ([`Self::check_prompt`]).
    pub prompt: Option<String>,
    /// The idempotency key. The owner derives it from what it delivers,
    /// never from the attempt.
    pub send_id: String,
    /// A legacy mode string. Absent means [`Delivery::Queue`], unless the
    /// wire `delivery` was a list, which is [`Self::delivery_order`].
    pub delivery: Option<Delivery>,
    /// The ordered list of the `delivery_order` group, carried on the wire
    /// as an array in `delivery` itself. A runner without the group refuses
    /// it naming `delivery` ([`Self::check_delivery_order`]), and an older
    /// runner's strict decode of `delivery` refuses an array the same way,
    /// so a list never goes out as a legacy `queue` by mistake. At most one
    /// of `delivery` and `delivery_order` is set.
    pub delivery_order: Option<Vec<crate::delivery_order::Mode>>,
    /// The session's fetch plan, carried verbatim when the runner declares
    /// `plans`. The first send establishes the session's plan and freezes its
    /// fetched manifest. An identical later repeat does not fetch again;
    /// a different plan is refused as `plan_drift` so an ordinary send cannot
    /// replace the session's frozen manifest.
    pub plan: Option<Map<String, Value>>,
    /// Whole-session retention in seconds. Frozen on the first send;
    /// subsequent sends may only shorten it. Absent initially means forever.
    pub retention: Option<u64>,
    /// The runner's own send parameters, kept as received.
    pub runner_params: Map<String, Value>,
}

impl SendRequest {
    /// A send of `prompt`.
    pub fn new(prompt: impl Into<String>, send_id: impl Into<String>) -> Self {
        Self {
            prompt: Some(prompt.into()),
            send_id: send_id.into(),
            delivery: None,
            delivery_order: None,
            plan: None,
            retention: None,
            runner_params: Map::new(),
        }
    }

    /// A template send built from `order`: it carries no `prompt`, sends
    /// `order.modes` as the list in `delivery`, and puts `order`'s
    /// `dedupe_key`, `expires_at_ms` and `template` in `runner_params`.
    pub fn template(
        send_id: impl Into<String>,
        order: &crate::delivery_order::DeliveryOrder,
    ) -> Self {
        let mut request = Self::new(String::new(), send_id).with_delivery_order(order);
        request.prompt = None;
        request
    }

    /// The prompt, or `""` for a send without one.
    pub fn prompt_text(&self) -> &str {
        self.prompt.as_deref().unwrap_or_default()
    }

    /// The `invalid_params` field for a send whose `prompt` does not match
    /// its kind, or `Ok`. A send naming a `template` (a list member, read
    /// from `runner_params`) carries no `prompt` at all: the runner renders
    /// its one text, and a `prompt` beside it, even an empty one, would be a
    /// second text. Every other send carries a non-empty `prompt`. Both
    /// refusals name `template`, since the template is what decides which
    /// shape the send must have.
    pub fn check_prompt(&self) -> Result<(), &'static str> {
        let template = self
            .runner_params
            .contains_key(crate::delivery_order::TEMPLATE_FIELD);
        let prompt = self.prompt.as_deref().is_some_and(|text| !text.is_empty());
        let ok = if template {
            self.prompt.is_none()
        } else {
            prompt
        };
        if ok {
            Ok(())
        } else {
            Err(crate::delivery_order::TEMPLATE_FIELD)
        }
    }

    /// A legacy mode string, replacing any list.
    pub fn with_delivery(mut self, delivery: Delivery) -> Self {
        self.delivery = Some(delivery);
        self.delivery_order = None;
        self
    }

    pub fn with_plan(mut self, plan: Map<String, Value>) -> Self {
        self.plan = Some(plan);
        self
    }

    pub fn with_retention(mut self, seconds: u64) -> Self {
        self.retention = Some(seconds);
        self
    }

    /// Replace the raw `runner_params["budget"]` value with this budget.
    pub fn with_budget(mut self, budget: Budget) -> Self {
        self.runner_params.insert(
            "budget".into(),
            serde_json::to_value(budget).expect("Budget contains only JSON values"),
        );
        self
    }

    /// Read a typed budget without moving or changing its raw value in
    /// `runner_params`. Returns the field path for an `invalid_params` refusal;
    /// decoding a send alone leaves legacy runner parameters untouched.
    pub fn budget(&self) -> Result<Option<Budget>, String> {
        let Some(raw) = self.runner_params.get("budget") else {
            return Ok(None);
        };
        let members = raw.as_object().ok_or_else(|| "budget".to_owned())?;
        if let Some(limit) = members.get("max_input_tokens") {
            if limit.as_u64().is_none_or(|limit| limit == 0) {
                return Err("budget.max_input_tokens".into());
            }
        }
        serde_json::from_value(raw.clone())
            .map(Some)
            .map_err(|_| "budget".into())
    }

    /// Read the positive model-response limit without changing raw parameters.
    /// An error is the offending path for an `invalid_params` refusal.
    pub fn max_steps(&self) -> Result<Option<u64>, String> {
        self.positive_limit("stop_when", "max_steps")
    }

    /// Read the positive per-response output cap, preserving the raw value.
    pub fn max_output_tokens(&self) -> Result<Option<u64>, String> {
        self.positive_limit("generation", "max_output_tokens")
    }

    fn positive_limit(&self, parent: &str, member: &str) -> Result<Option<u64>, String> {
        let Some(raw) = self.runner_params.get(parent) else {
            return Ok(None);
        };
        let object = raw.as_object().ok_or_else(|| parent.to_owned())?;
        object.get(member).map_or(Ok(None), |value| {
            value
                .as_u64()
                .filter(|limit| *limit > 0)
                .map(Some)
                .ok_or_else(|| format!("{parent}.{member}"))
        })
    }

    /// Whether `tools` explicitly requests no tools. Absence is `None`;
    /// a non-empty, runner-specific list is `Some(false)`. A non-list names `tools`.
    pub fn no_tools(&self) -> Result<Option<bool>, String> {
        self.runner_params.get("tools").map_or(Ok(None), |value| {
            value
                .as_array()
                .map(|tools| Some(tools.is_empty()))
                .ok_or_else(|| "tools".to_owned())
        })
    }

    /// Set `stop_when.max_steps`, preserving other `stop_when` members.
    pub fn with_max_steps(mut self, limit: u64) -> Self {
        self.set_limit("stop_when", "max_steps", limit);
        self
    }

    /// Set `generation.max_output_tokens`, preserving other generation settings.
    pub fn with_max_output_tokens(mut self, limit: u64) -> Self {
        self.set_limit("generation", "max_output_tokens", limit);
        self
    }

    fn set_limit(&mut self, parent: &str, member: &str, limit: u64) {
        let raw = self
            .runner_params
            .entry(parent)
            .or_insert_with(|| Value::Object(Map::new()));
        if !raw.is_object() {
            *raw = Value::Object(Map::new());
        }
        raw.as_object_mut()
            .expect("limit parent is an object")
            .insert(member.into(), Value::from(limit));
    }

    /// Offer no tools, including plan tools and host built-ins.
    pub fn with_no_tools(mut self) -> Self {
        self.runner_params
            .insert("tools".into(), Value::Array(vec![]));
        self
    }

    /// Replace the raw `runner_params["structured_output"]` value with this
    /// request. The member stays in `runner_params`, as it arrives on the wire.
    pub fn with_structured_output(
        mut self,
        structured_output: crate::structured_output::StructuredOutput,
    ) -> Self {
        self.runner_params.insert(
            crate::structured_output::FIELD.into(),
            serde_json::to_value(structured_output)
                .expect("StructuredOutput contains only JSON values"),
        );
        self
    }

    /// Read a typed `structured_output` without moving or changing its raw
    /// value in `runner_params`. `Ok(None)` when the send carries none. On a
    /// malformed value, returns the field path of the `invalid_params`
    /// refusal: `structured_output` for a non-object, `structured_output.mode`
    /// for a missing or unknown mode, `structured_output.name` for a name
    /// outside `[a-z0-9_]{1,48}`, `structured_output.schema` for a schema
    /// that is not an object, references outside its document, names a
    /// dialect other than draft 2020-12, or exceeds 65,536 JCS bytes, and
    /// `structured_output.<member>` for an unknown member. Checks that depend
    /// on the runner and the session are
    /// [`StructuredOutput::check`](crate::structured_output::StructuredOutput::check).
    pub fn structured_output(
        &self,
    ) -> Result<Option<crate::structured_output::StructuredOutput>, String> {
        self.runner_params
            .get(crate::structured_output::FIELD)
            .map(crate::structured_output::decode)
            .transpose()
    }

    /// Validate retention before writing anything. `previous` is `None`
    /// for a fresh lineage and `Some(frozen_retention)` for an existing one,
    /// including `Some(None)` for one first sent without retention. Absence
    /// on a later send preserves the current policy, never resets it.
    /// Expiry of an old lineage must be resolved before this check.
    /// Pass `None` for `limits` when the capability group is undeclared,
    /// even if an undeclared limits object was supplied in discovery.
    pub fn check_retention(
        &self,
        limits: Option<&crate::describe::Retention>,
        previous: Option<Option<u64>>,
    ) -> Result<(), crate::errors::RetentionInvalidDetail> {
        use crate::errors::RetentionInvalidDetail;
        let Some(seconds) = self.retention else {
            return Ok(());
        };
        let Some(limits) = limits else {
            return Err(RetentionInvalidDetail::new());
        };
        if seconds == 0 {
            return Err(RetentionInvalidDetail::new());
        }
        if seconds > limits.max_seconds {
            return Err(RetentionInvalidDetail::new().with_max_seconds(limits.max_seconds));
        }
        if let Some(frozen) = previous {
            if frozen.is_none_or(|value| seconds > value) {
                return Err(RetentionInvalidDetail::new());
            }
        }
        Ok(())
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

    /// The `delivery_order` send: its list (the array in `delivery`) with the
    /// members that ride beside it (`dedupe_key`, `expires_at_ms`,
    /// `template`, read from `runner_params` without moving them). `Ok(None)`
    /// for a send without a list. On a malformed send, the field of the
    /// `invalid_params` refusal: a member sent without a list (with a string
    /// or absent `delivery`) is refused naming that member, because an older
    /// runner would ignore it, while it refuses the whole send when the list
    /// rides in `delivery`. The `prompt` must match the send's kind
    /// ([`Self::check_prompt`]), or the send is refused naming `template`.
    pub fn delivery_order(
        &self,
    ) -> Result<Option<crate::delivery_order::DeliveryOrder>, &'static str> {
        let order = crate::delivery_order::DeliveryOrder::decode(
            self.delivery_order.as_deref(),
            &self.runner_params,
        )?;
        if order.is_some() {
            self.check_prompt()?;
        }
        Ok(order)
    }

    /// A runner that does not declare `delivery_order` (in `declared`, its
    /// module-level capabilities) refuses a send whose `delivery` is a list
    /// with `invalid_params` naming `delivery`, as an older runner's strict
    /// decode does: it never delivers the list as a legacy mode. A runner
    /// that declares the group passes; it checks the members with
    /// [`Self::delivery_order`].
    pub fn check_delivery_order<S: AsRef<str>>(&self, declared: &[S]) -> Result<(), &'static str> {
        let declares = declared
            .iter()
            .any(|name| name.as_ref() == crate::capabilities::DELIVERY_ORDER);
        if self.delivery_order.is_some() && !declares {
            Err(DELIVERY_FIELD)
        } else {
            Ok(())
        }
    }

    /// Make this a `delivery_order` send: `order`'s list in `delivery`
    /// (replacing any legacy mode) and its members in `runner_params`.
    pub fn with_delivery_order(mut self, order: &crate::delivery_order::DeliveryOrder) -> Self {
        for member in crate::delivery_order::MEMBERS {
            self.runner_params.remove(*member);
        }
        self.delivery = None;
        self.delivery_order = Some(order.modes.clone());
        self.runner_params.extend(order.to_members());
        self
    }

    /// Admission's check of the plan's `render_tags` against `declared` (a
    /// runner's module-level capabilities): `Err` with the `field` of an
    /// `invalid_params` refusal (`plan.render_tags`) for a value that is not
    /// a boolean, or `true` on a runner without `tags`. A request without a
    /// plan passes.
    pub fn check_render_tags<S: AsRef<str>>(&self, declared: &[S]) -> Result<(), &'static str> {
        match &self.plan {
            Some(plan) => crate::tags::check_plan(plan, declared),
            None => Ok(()),
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
    /// The run is paused; `pause_reason` may explain why.
    pub const PAUSED: &str = "paused";
    /// A `delivery_order` send replaced, while still queued and undelivered,
    /// by a later send from the same sender with the same `dedupe_key`.
    /// Terminal: it is never delivered. A re-send of its `send_id` answers
    /// this state.
    pub const SUPERSEDED: &str = "superseded";
    /// A `delivery_order` send dropped, undelivered, at its `expires_at_ms`.
    /// Terminal: it is never delivered. A re-send of its `send_id` answers
    /// this state.
    pub const EXPIRED: &str = "expired";
}

/// A send state, classified without rejecting a runner's additional states.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SendState {
    Active,
    Finished,
    Pending,
    Paused,
    Superseded,
    Expired,
    Unknown(String),
}
impl SendState {
    pub fn parse(value: &str) -> Self {
        match value {
            states::ACTIVE => Self::Active,
            states::FINISHED => Self::Finished,
            states::PENDING => Self::Pending,
            states::PAUSED => Self::Paused,
            states::SUPERSEDED => Self::Superseded,
            states::EXPIRED => Self::Expired,
            other => Self::Unknown(other.into()),
        }
    }
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
    /// The id of a queued submission, on a `pending` reply. Opaque: a caller
    /// never derives a run id from it. Once the send is delivered, `run_id`
    /// names the run that carried it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// A paused reply's reason, distinct from the run's terminal `reason`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    /// On the reply to the send that admitted the session: what was frozen,
    /// read from the record the runner wrote. A retry of that send with the
    /// same `send_id` gets the same facts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Baseline>,
    /// Steer delivery receipt, when applicable. For a queued send once
    /// delivered: `step` with the stored row as `ref` when it went into a
    /// running run, `turn` with the run as `ref` when it started one, and
    /// `unknown` when delivery cannot be confirmed, each with `run_id` naming
    /// that run when known. These three are durable: a restart never turns
    /// them back into `pending` or absent.
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
            pause_reason: None,
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

    pub fn with_pause_reason(mut self, reason: impl Into<String>) -> Self {
        self.pause_reason = Some(reason.into());
        self
    }

    pub fn send_state(&self) -> SendState {
        SendState::parse(&self.state)
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
    fn paused_send_bytes_preserve_pause_reason_and_open_states() {
        let absent = SendReply::new(states::PAUSED);
        assert_eq!(
            serde_json::to_string(&absent).unwrap(),
            r#"{"state":"paused"}"#
        );
        let full = absent.with_run_id("r1").with_pause_reason("future-pause");
        let bytes = r#"{"state":"paused","run_id":"r1","pause_reason":"future-pause"}"#;
        assert_eq!(serde_json::to_string(&full).unwrap(), bytes);
        assert_eq!(serde_json::from_str::<SendReply>(bytes).unwrap(), full);
        assert_eq!(full.send_state(), SendState::Paused);
        let future = serde_json::from_str::<SendReply>(r#"{"state":"future"}"#).unwrap();
        assert_eq!(future.send_state(), SendState::Unknown("future".into()));
        let request = serde_json::from_str::<SendRequest>(
            r#"{"prompt":"p","send_id":"s","mark":"runner-owned"}"#,
        )
        .unwrap();
        assert_eq!(request.runner_params["mark"], "runner-owned");
    }

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

#[cfg(test)]
mod retention_tests {
    use super::SendRequest;
    use crate::describe::Retention;
    use serde_json::json;

    #[test]
    fn retention_is_a_role_field_and_round_trips() {
        let request: SendRequest = serde_json::from_value(
            json!({"prompt":"p", "send_id":"s", "retention":3, "model":"m"}),
        )
        .unwrap();
        assert_eq!(request.retention, Some(3));
        assert!(!request.runner_params.contains_key("retention"));
        assert_eq!(serde_json::to_value(&request).unwrap()["retention"], 3);
        for bad in [json!(-1), json!(1.5), json!("3"), json!(true)] {
            let error = serde_json::from_value::<SendRequest>(
                json!({"prompt":"p", "send_id":"s", "retention":bad}),
            )
            .unwrap_err();
            assert!(error.to_string().contains("u64"), "{error}");
            assert!(error.to_string().contains("retention"), "{error}");
        }
    }

    #[test]
    fn retention_validation_freezes_and_only_shortens() {
        let limits = Retention::new(10, 500);
        let send = |value| SendRequest::new("p", "s").with_retention(value);
        assert!(send(10).check_retention(Some(&limits), None).is_ok());
        assert!(send(5)
            .check_retention(Some(&limits), Some(Some(10)))
            .is_ok());
        assert!(send(5)
            .check_retention(Some(&limits), Some(Some(5)))
            .is_ok());
        assert_eq!(
            send(6)
                .check_retention(Some(&limits), Some(Some(5)))
                .unwrap_err()
                .field,
            "retention"
        );
        assert!(send(1).check_retention(Some(&limits), Some(None)).is_err());
        assert!(send(1).check_retention(None, None).is_err());
        assert!(send(0).check_retention(Some(&limits), None).is_err());
        assert_eq!(
            send(11)
                .check_retention(Some(&limits), None)
                .unwrap_err()
                .max_seconds,
            Some(10)
        );
        assert!(SendRequest::new("p", "s")
            .check_retention(None, Some(Some(5)))
            .is_ok());
    }
}

#[cfg(test)]
mod input_cap_tests {
    use super::*;
    use crate::{errors, vectors};

    #[test]
    fn legacy_budget_stays_in_runner_params_byte_for_byte() {
        let file = vectors::load("input-token-cap.json");
        let case = &file["legacy_budget"];
        let expected_bytes = case["budget_bytes"].as_str().unwrap().as_bytes();
        let request: SendRequest = vectors::round_trip("legacy budget", &case["params"]);
        // Legacy consumers read this exact map entry, not the typed accessor.
        assert_eq!(
            serde_json::to_vec(&request.runner_params["budget"]).unwrap(),
            expected_bytes
        );
        let budget = request.budget().unwrap().unwrap();
        assert_eq!(budget.max_input_tokens, None);
        assert_eq!(budget.runner_params["finalize_text"], "x");
        assert_eq!(
            budget.runner_params["nudges"],
            serde_json::json!([{"at_tool_calls":1,"text":"hint"}])
        );
        assert_eq!(serde_json::to_vec(&budget).unwrap(), expected_bytes);
        assert_eq!(
            serde_json::to_vec(&request.runner_params["budget"]).unwrap(),
            expected_bytes
        );
    }

    #[test]
    fn with_budget_replaces_raw_value_without_duplicate_keys() {
        let mut request = SendRequest::new("prompt", "key");
        assert_eq!(request.budget().unwrap(), None);
        request
            .runner_params
            .insert("budget".into(), serde_json::json!({"finalize_text":"old"}));
        let budget = Budget::new().with_max_input_tokens(7);
        let request = request.with_budget(budget.clone());
        assert_eq!(request.budget().unwrap(), Some(budget));
        assert_eq!(
            request.runner_params["budget"],
            serde_json::json!({"max_input_tokens":7})
        );
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(encoded.matches("\"budget\":").count(), 1, "{encoded}");
        assert!(!encoded.contains("old"));
    }

    #[test]
    fn input_cap_send_vectors_accept_and_refuse_by_field() {
        let file = vectors::load("input-token-cap.json");
        for case in vectors::cases(&file, "accept") {
            let request: SendRequest =
                vectors::round_trip(case["name"].as_str().unwrap(), &case["params"]);
            request
                .budget()
                .unwrap()
                .unwrap()
                .check(
                    case["advertised"].as_bool().unwrap(),
                    case["frozen"].as_u64(),
                )
                .unwrap();
        }
        for case in vectors::cases(&file, "refuse") {
            let field = case["error"]["detail"]["field"].as_str().unwrap();
            assert_eq!(case["error"]["code"], errors::INVALID_PARAMS);
            let request: SendRequest =
                vectors::round_trip(case["name"].as_str().unwrap(), &case["params"]);
            let result = request.budget().and_then(|budget| {
                budget.unwrap().check(
                    case["advertised"].as_bool().unwrap(),
                    case["frozen"].as_u64(),
                )
            });
            assert_eq!(result, Err(field.into()), "{}", case["name"]);
        }
        assert!(Budget::new()
            .with_max_input_tokens(0)
            .check(true, None)
            .is_err());
        assert!(SendRequest::new("prompt", "key")
            .with_budget(Budget::new().with_max_input_tokens(1))
            .budget()
            .unwrap()
            .is_some());
    }
}

#[cfg(test)]
mod prompt_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_a_template_send_omits_its_prompt() {
        let plain: SendRequest =
            serde_json::from_value(json!({"prompt": "hi", "send_id": "s"})).unwrap();
        assert_eq!(plain.check_prompt(), Ok(()));
        assert_eq!(plain.prompt_text(), "hi");
        for request in [
            json!({"send_id": "s"}),
            json!({"prompt": "", "send_id": "s"}),
        ] {
            let request: SendRequest = serde_json::from_value(request).unwrap();
            assert_eq!(
                request.check_prompt(),
                Err(crate::delivery_order::TEMPLATE_FIELD)
            );
        }
        let template = json!({"id": "t", "digest": "d"});
        let bare: SendRequest = serde_json::from_value(
            json!({"send_id": "s", "delivery": ["steer"], "template": template}),
        )
        .unwrap();
        assert_eq!(bare.check_prompt(), Ok(()));
        assert_eq!(serde_json::to_value(&bare).unwrap().get("prompt"), None);
        for prompt in ["", "words"] {
            let carrying: SendRequest = serde_json::from_value(
                json!({"prompt": prompt, "send_id": "s", "delivery": ["steer"], "template": template}),
            )
            .unwrap();
            assert_eq!(
                carrying.check_prompt(),
                Err(crate::delivery_order::TEMPLATE_FIELD),
                "{prompt:?}"
            );
        }
    }
}

#[cfg(test)]
mod run_parameter_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_parameter_vectors_preserve_bytes_and_name_refusals() {
        let file = crate::vectors::load("run-parameters.json");
        let request: SendRequest = crate::vectors::round_trip("run parameters", &file["send"]);
        assert_eq!(request.max_steps(), Ok(Some(2)));
        assert_eq!(request.max_output_tokens(), Ok(Some(16)));
        assert_eq!(request.no_tools(), Ok(Some(true)));
        assert_eq!(
            serde_json::to_string(&request.runner_params).unwrap(),
            file["runner_params_bytes"].as_str().unwrap()
        );
        for case in file["refuse"].as_array().unwrap() {
            let request: SendRequest =
                crate::vectors::round_trip("malformed run parameter", &case["params"]);
            let field = if request.runner_params.contains_key("stop_when") {
                request.max_steps().unwrap_err()
            } else if request.runner_params.contains_key("generation") {
                request.max_output_tokens().unwrap_err()
            } else {
                request.no_tools().unwrap_err()
            };
            assert_eq!(case["error"]["code"], crate::errors::INVALID_PARAMS);
            assert_eq!(field, case["error"]["detail"]["field"].as_str().unwrap());
        }
    }

    #[test]
    fn run_parameter_groups_are_independent_and_require_send() {
        let file = crate::vectors::load("run-parameters-describe.json");
        let groups = [
            crate::capabilities::MAX_STEPS,
            crate::capabilities::MAX_OUTPUT_TOKENS,
            crate::capabilities::NO_TOOLS,
        ];
        for group in groups {
            let mut description = file.clone();
            description["capabilities"] = json!([group]);
            assert!(crate::describe::check_describe(&description).is_ok());
            description["majors"][0]["ops"] = json!(["role.describe", "session.baseline"]);
            assert!(crate::describe::check_describe(&description).is_err());
        }
        assert!(crate::describe::check_describe(&file).is_ok());
    }

    #[test]
    fn run_parameters_round_trip_byte_for_byte() {
        let bytes = br#"{"generation":{"max_output_tokens":16,"temperature":0.25},"stop_when":{"max_steps":2,"other":"kept"},"tools":[],"vendor":{"x":true}}"#;
        let raw: Map<String, Value> = serde_json::from_slice(bytes).unwrap();
        let request = SendRequest::new("hello", "limits").with_runner_params(raw);
        let wire = serde_json::to_vec(&request).unwrap();
        let decoded: SendRequest = serde_json::from_slice(&wire).unwrap();
        assert_eq!(decoded.max_steps(), Ok(Some(2)));
        assert_eq!(decoded.max_output_tokens(), Ok(Some(16)));
        assert_eq!(decoded.no_tools(), Ok(Some(true)));
        assert_eq!(serde_json::to_vec(&decoded.runner_params).unwrap(), bytes);
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), wire);
    }

    #[test]
    fn run_parameter_builders_preserve_siblings_and_replace_single_values() {
        let request = SendRequest::new("hello", "limits").with_runner_params(
            json!({"stop_when":{"other":true},"generation":{"temperature":0.5},"tools":["native"]})
                .as_object().unwrap().clone(),
        ).with_max_steps(2).with_max_output_tokens(16).with_no_tools();
        assert_eq!(
            request.runner_params,
            json!({
                "stop_when":{"other":true,"max_steps":2},
                "generation":{"temperature":0.5,"max_output_tokens":16},"tools":[]
            })
            .as_object()
            .unwrap()
            .clone()
        );
        assert_eq!(request.max_steps(), Ok(Some(2)));
        assert_eq!(request.max_output_tokens(), Ok(Some(16)));
        assert_eq!(request.no_tools(), Ok(Some(true)));
    }

    #[test]
    fn run_parameter_accessors_name_malformed_values_without_changing_them() {
        for (parent, member) in [
            ("stop_when", "max_steps"),
            ("generation", "max_output_tokens"),
        ] {
            for value in [
                json!(0),
                json!(-1),
                json!(1.5),
                json!(null),
                json!("2"),
                json!(true),
                json!(18446744073709551616.0),
            ] {
                let raw = json!({parent: {member: value}});
                let request: SendRequest = serde_json::from_value(
                    json!({"prompt":"hi", "send_id":"bad", parent: raw[parent]}),
                )
                .unwrap();
                let result = if parent == "stop_when" {
                    request.max_steps()
                } else {
                    request.max_output_tokens()
                };
                assert_eq!(result, Err(format!("{parent}.{member}")));
                assert_eq!(request.runner_params[parent], raw[parent]);
            }
            let request = SendRequest::new("hi", "bad")
                .with_runner_params(json!({parent: null}).as_object().unwrap().clone());
            let result = if parent == "stop_when" {
                request.max_steps()
            } else {
                request.max_output_tokens()
            };
            assert_eq!(result, Err(parent.into()));
        }
        let absent = SendRequest::new("hi", "absent");
        assert_eq!(absent.max_steps(), Ok(None));
        assert_eq!(absent.max_output_tokens(), Ok(None));
        assert_eq!(absent.no_tools(), Ok(None));
        for value in [json!(null), json!({}), json!(false), json!("none")] {
            let request = SendRequest::new("hi", "bad")
                .with_runner_params(json!({"tools":value}).as_object().unwrap().clone());
            assert_eq!(request.no_tools(), Err("tools".into()));
        }
        let request = SendRequest::new("hi", "specific")
            .with_runner_params(json!({"tools":["native"]}).as_object().unwrap().clone());
        assert_eq!(request.no_tools(), Ok(Some(false)));
    }
}
