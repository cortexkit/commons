//! The `delivery_order` group: an ordered list of delivery modes on
//! `session.send`, the senders a session's plan admits, and the templates
//! whose text may go out with system authority.
//!
//! A runner that declares `delivery_order`
//! ([`crate::capabilities::DELIVERY_ORDER`]) accepts `delivery_order:
//! [mode, ...]` on a send ([`DeliveryOrder`]). At each request boundary it
//! takes the first mode in the list that is possible there ([`choose`]); if
//! none is, the send stays queued, durably, for the next boundary. A list
//! without `idle_start` never starts a run.
//!
//! The legacy string `delivery` is unchanged and maps onto lists
//! ([`legacy_equivalent`]): `queue` (or absent) is `[idle_start]`, `steer` is
//! `[steer, idle_start]`, and `interrupt` has no list form. A send carrying
//! both is refused naming `delivery_order`.
//!
//! `tool_result`, `steer` and `idle_start` deliver at user authority.
//! `turn_system` delivers a turn-scoped system message, and only for text the
//! runner rendered itself from a template the session's plan declares for
//! that sender, with every typed slot valid ([`render`]); otherwise it is not
//! possible and the runner falls through to the next mode.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{capabilities, errors, send::Delivery};

/// The send member holding the ordered list, and the `field` an
/// `invalid_params` refusal names for a malformed list, a list beside a
/// legacy `delivery`, or a mode the sender's grant excludes.
pub const FIELD: &str = "delivery_order";
/// The send member naming a queued send's replacement key.
pub const DEDUPE_KEY_FIELD: &str = "dedupe_key";
/// The send member giving the time after which a queued send is dropped.
pub const EXPIRES_AT_FIELD: &str = "expires_at_ms";
/// The send member naming the plan template a `turn_system` delivery renders.
pub const TEMPLATE_FIELD: &str = "template";

/// One delivery mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Mode {
    /// Rendered with the next tool result, the first time that result is
    /// sent. The stored tool result never changes: the runner records that
    /// the send was delivered with that result. User authority.
    ToolResult,
    /// A turn-scoped system message (Anthropic's `clear_at:
    /// next_user_message`), for a plan template's rendering only.
    TurnSystem,
    /// A user message into the run in progress, at its next step boundary.
    Steer,
    /// A user message that starts a run on an idle session.
    IdleStart,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ToolResult => modes::TOOL_RESULT,
            Self::TurnSystem => modes::TURN_SYSTEM,
            Self::Steer => modes::STEER,
            Self::IdleStart => modes::IDLE_START,
        }
    }

    /// Whether the mode delivers at user authority: every mode but
    /// `turn_system`.
    pub fn is_user_authority(self) -> bool {
        self != Self::TurnSystem
    }
}

/// The modes' spellings, as a read entry's `delivered_as.mode` names them
/// (decoded open there).
pub mod modes {
    pub const TOOL_RESULT: &str = "tool_result";
    pub const TURN_SYSTEM: &str = "turn_system";
    pub const STEER: &str = "steer";
    pub const IDLE_START: &str = "idle_start";
}

/// The list a legacy string `delivery` stands for: `queue` (what an absent
/// `delivery` means) waits durably and starts the next turn once the run
/// ends, `[idle_start]`; `steer` is `[steer, idle_start]`. `interrupt` has
/// no list form: it stays a mode of its own.
pub fn legacy_equivalent(delivery: Delivery) -> Option<Vec<Mode>> {
    match delivery {
        Delivery::Queue => Some(vec![Mode::IdleStart]),
        Delivery::Steer => Some(vec![Mode::Steer, Mode::IdleStart]),
        Delivery::Interrupt => None,
    }
}

/// A send's use of a plan template: which template, the digest the sender
/// expects it to have, and its slot values.
///
/// Strict: an unknown member is refused naming `template`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TemplateUse {
    pub id: String,
    /// The template's digest ([`template_digest`]) as the sender knows it.
    pub digest: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub slots: Map<String, Value>,
}

impl TemplateUse {
    pub fn new(
        id: impl Into<String>,
        digest: impl Into<String>,
        slots: Map<String, Value>,
    ) -> Self {
        Self {
            id: id.into(),
            digest: digest.into(),
            slots,
        }
    }
}

/// The `delivery_order` members of a send, read by
/// [`crate::send::SendRequest::delivery_order`]. They ride beside the role's
/// members in `runner_params`, as `budget` does.
///
/// Non-exhaustive: build one with [`DeliveryOrder::new`] and the `with_*`
/// setters.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DeliveryOrder {
    /// The modes, in the order the runner tries them. Never empty, no mode
    /// twice.
    pub modes: Vec<Mode>,
    /// A queued, undelivered send from the same sender with the same key is
    /// replaced by this one, and ends `superseded`.
    pub dedupe_key: Option<String>,
    /// Milliseconds since the Unix epoch after which the send, if still
    /// undelivered, is dropped durably and ends `expired`.
    pub expires_at_ms: Option<u64>,
    /// The plan template a `turn_system` delivery renders.
    pub template: Option<TemplateUse>,
}

impl DeliveryOrder {
    pub fn new(modes: Vec<Mode>) -> Self {
        Self {
            modes,
            dedupe_key: None,
            expires_at_ms: None,
            template: None,
        }
    }

    pub fn with_dedupe_key(mut self, key: impl Into<String>) -> Self {
        self.dedupe_key = Some(key.into());
        self
    }

    pub fn with_expires_at_ms(mut self, at: u64) -> Self {
        self.expires_at_ms = Some(at);
        self
    }

    pub fn with_template(mut self, template: TemplateUse) -> Self {
        self.template = Some(template);
        self
    }

    /// Whether the list can start a run: only `idle_start` does.
    pub fn may_start_a_run(&self) -> bool {
        self.modes.contains(&Mode::IdleStart)
    }

    /// The members as they ride on a send.
    pub fn to_members(&self) -> Map<String, Value> {
        let mut members = Map::new();
        members.insert(
            FIELD.into(),
            serde_json::to_value(&self.modes).expect("modes serialize"),
        );
        if let Some(key) = &self.dedupe_key {
            members.insert(DEDUPE_KEY_FIELD.into(), Value::from(key.as_str()));
        }
        if let Some(at) = self.expires_at_ms {
            members.insert(EXPIRES_AT_FIELD.into(), Value::from(at));
        }
        if let Some(template) = &self.template {
            members.insert(
                TEMPLATE_FIELD.into(),
                serde_json::to_value(template).expect("template serializes"),
            );
        }
        members
    }

    /// Read the members from a send's `runner_params`. `legacy` is whether
    /// the send also carries a legacy `delivery`. `Ok(None)` when the send
    /// carries none of them. On a malformed send, the field of the
    /// `invalid_params` refusal: [`FIELD`] for a list beside a legacy
    /// `delivery`, a list that is not an array of known modes, an empty list
    /// or a mode twice; [`DEDUPE_KEY_FIELD`], [`EXPIRES_AT_FIELD`] or
    /// [`TEMPLATE_FIELD`] for a malformed member, or for one sent without a
    /// list.
    pub fn decode(params: &Map<String, Value>, legacy: bool) -> Result<Option<Self>, &'static str> {
        let Some(raw) = params.get(FIELD) else {
            for member in [DEDUPE_KEY_FIELD, EXPIRES_AT_FIELD, TEMPLATE_FIELD] {
                if params.contains_key(member) {
                    return Err(member);
                }
            }
            return Ok(None);
        };
        if legacy {
            return Err(FIELD);
        }
        let modes: Vec<Mode> = serde_json::from_value(raw.clone()).map_err(|_| FIELD)?;
        let mut seen = modes.clone();
        seen.sort();
        seen.dedup();
        if modes.is_empty() || seen.len() != modes.len() {
            return Err(FIELD);
        }
        let mut order = Self::new(modes);
        if let Some(key) = params.get(DEDUPE_KEY_FIELD) {
            match key.as_str() {
                Some(key) if !key.is_empty() => order.dedupe_key = Some(key.to_owned()),
                _ => return Err(DEDUPE_KEY_FIELD),
            }
        }
        if let Some(at) = params.get(EXPIRES_AT_FIELD) {
            order.expires_at_ms = Some(at.as_u64().ok_or(EXPIRES_AT_FIELD)?);
        }
        if let Some(template) = params.get(TEMPLATE_FIELD) {
            order.template =
                Some(serde_json::from_value(template.clone()).map_err(|_| TEMPLATE_FIELD)?);
        }
        Ok(Some(order))
    }
}

/// What is possible at one request boundary, as the runner finds it.
///
/// Non-exhaustive: build one with [`Boundary::default`] and the `with_*`
/// setters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Boundary {
    /// A tool result is about to be sent for the first time.
    pub tool_result: bool,
    /// The provider supports a turn-scoped system message, the runner owns
    /// the request, and the send's template rendered with valid slots.
    pub turn_system: bool,
    /// A run is in progress and at its next step.
    pub run_in_progress: bool,
    /// The session is idle: no run in progress.
    pub idle: bool,
}

impl Boundary {
    pub fn with_tool_result(mut self) -> Self {
        self.tool_result = true;
        self
    }
    pub fn with_turn_system(mut self) -> Self {
        self.turn_system = true;
        self
    }
    pub fn with_run_in_progress(mut self) -> Self {
        self.run_in_progress = true;
        self
    }
    pub fn with_idle(mut self) -> Self {
        self.idle = true;
        self
    }

    pub fn possible(&self, mode: Mode) -> bool {
        match mode {
            Mode::ToolResult => self.tool_result,
            Mode::TurnSystem => self.turn_system,
            Mode::Steer => self.run_in_progress,
            Mode::IdleStart => self.idle,
        }
    }
}

/// The mode a queued send is delivered in at `boundary`: the first in
/// `modes` that is possible there, or `None`, and the send stays queued.
pub fn choose(modes: &[Mode], boundary: &Boundary) -> Option<Mode> {
    modes.iter().copied().find(|mode| boundary.possible(*mode))
}

/// How a delivered send went out, on its transcript entry
/// ([`crate::read::ReadMessage::delivered_as`]), so readers and a resumed run
/// render exactly what was sent.
///
/// Non-exhaustive: build one with [`EntryDelivery::new`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct EntryDelivery {
    /// One of [`modes`], decoded open.
    pub mode: String,
    /// For `tool_result`: the `call_key` of the tool result it went out
    /// with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_key: Option<String>,
}

impl EntryDelivery {
    pub fn new(mode: Mode) -> Self {
        Self {
            mode: mode.as_str().to_owned(),
            call_key: None,
        }
    }

    pub fn with_call_key(mut self, call_key: impl Into<String>) -> Self {
        self.call_key = Some(call_key.into());
        self
    }
}

/// The plan member listing the modules, other than the owner, whose sends
/// the session admits, and the `field` of its `invalid_params` refusal.
pub const PLAN_SENDERS: &str = "senders";
pub const PLAN_SENDERS_FIELD: &str = "plan.senders";

/// A typed template slot. There are no string slots.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Slot {
    /// An integer from `min` to `max`, both inclusive.
    Integer { min: i64, max: i64 },
    /// An integer from 0 to 100.
    Percentage,
    /// A tag (`§N§`) the session assigned.
    Tag,
    /// One of `values`.
    Enum { values: Vec<String> },
}

/// A template a plan declares for one sender: its text, with `{name}`
/// placeholders, and the slot each placeholder takes.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Template {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub slots: BTreeMap<String, Slot>,
}

impl Template {
    pub fn new(
        id: impl Into<String>,
        text: impl Into<String>,
        slots: BTreeMap<String, Slot>,
    ) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            slots,
        }
    }
}

/// One module the plan admits as a sender.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PlanSender {
    /// The module, matched against the route's attested principal.
    pub module: String,
    /// Whether this sender may list `idle_start`, and so start a run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub idle_start: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<Template>,
}

impl PlanSender {
    pub fn new(module: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            idle_start: false,
            templates: Vec::new(),
        }
    }
    pub fn with_idle_start(mut self) -> Self {
        self.idle_start = true;
        self
    }
    pub fn with_templates(mut self, templates: Vec<Template>) -> Self {
        self.templates = templates;
        self
    }
}

/// The template's digest: lowercase hex SHA-256 over the RFC 8785 (JCS)
/// bytes of the template object as the plan carries it.
pub fn template_digest(template: &Template) -> String {
    let bytes = serde_jcs::to_vec(template).expect("a template serializes");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The placeholders of a template text, in order, or `None` when the text
/// has a `{` or `}` outside a `{name}` placeholder (`name` is `[a-z0-9_]+`).
fn placeholders(text: &str) -> Option<Vec<&str>> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find(['{', '}']) {
        if rest[open..].starts_with('}') {
            return None;
        }
        let after = &rest[open + 1..];
        let close = after.find('}')?;
        let name = &after[..close];
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return None;
        }
        names.push(name);
        rest = &after[close + 1..];
    }
    Some(names)
}

/// The plan's senders, or [`PLAN_SENDERS_FIELD`] when the member is malformed:
/// not an array of senders, a module listed twice, a template id repeated
/// within a sender, a placeholder naming no slot or a slot no placeholder
/// uses, an integer slot whose `min` is over its `max`, or an enum slot with
/// no value. Absent or `null` is no sender.
pub fn plan_senders(plan: &Map<String, Value>) -> Result<Vec<PlanSender>, &'static str> {
    let senders: Vec<PlanSender> = match plan.get(PLAN_SENDERS) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(raw) => serde_json::from_value(raw.clone()).map_err(|_| PLAN_SENDERS_FIELD)?,
    };
    let mut modules = std::collections::BTreeSet::new();
    for sender in &senders {
        if !modules.insert(sender.module.as_str()) {
            return Err(PLAN_SENDERS_FIELD);
        }
        let mut ids = std::collections::BTreeSet::new();
        for template in &sender.templates {
            if !ids.insert(template.id.as_str()) {
                return Err(PLAN_SENDERS_FIELD);
            }
            let used = placeholders(&template.text).ok_or(PLAN_SENDERS_FIELD)?;
            let used: std::collections::BTreeSet<&str> = used.into_iter().collect();
            let declared: std::collections::BTreeSet<&str> =
                template.slots.keys().map(String::as_str).collect();
            if used != declared {
                return Err(PLAN_SENDERS_FIELD);
            }
            for slot in template.slots.values() {
                match slot {
                    Slot::Integer { min, max } if min > max => return Err(PLAN_SENDERS_FIELD),
                    Slot::Enum { values } if values.is_empty() => return Err(PLAN_SENDERS_FIELD),
                    _ => {}
                }
            }
        }
    }
    Ok(senders)
}

/// Admission's check of a plan's `senders` against `declared`: a malformed
/// member, or a non-empty one on a runner without `delivery_order`, is
/// refused `invalid_params` naming [`PLAN_SENDERS_FIELD`], writing nothing.
pub fn check_plan<S: AsRef<str>>(
    plan: &Map<String, Value>,
    declared: &[S],
) -> Result<(), &'static str> {
    let senders = plan_senders(plan)?;
    if !senders.is_empty()
        && !declared
            .iter()
            .any(|name| name.as_ref() == capabilities::DELIVERY_ORDER)
    {
        return Err(PLAN_SENDERS_FIELD);
    }
    Ok(())
}

/// A send's admission on a runner that declares `delivery_order`. `caller`
/// is the route's attested principal, `owner` the session's recorded owner
/// (`None` before the first send), `senders` the plan's, and `modes` the
/// send's list (or its legacy equivalent). The owner is admitted with any
/// list. Any other caller is admitted only when the plan names it, and may
/// list `idle_start` only when its entry grants it. `Err` is the refusal:
/// `scope_owner_mismatch` with no field for an unnamed caller, or
/// `invalid_params` naming [`FIELD`] for an `idle_start` the grant excludes.
pub fn admit(
    caller: &str,
    owner: Option<&str>,
    senders: &[PlanSender],
    modes: &[Mode],
) -> Result<(), (&'static str, Option<&'static str>)> {
    if owner.is_none_or(|owner| owner == caller) {
        return Ok(());
    }
    let sender = senders
        .iter()
        .find(|sender| sender.module == caller)
        .ok_or((errors::SCOPE_OWNER_MISMATCH, None))?;
    if modes.contains(&Mode::IdleStart) && !sender.idle_start {
        return Err((errors::INVALID_PARAMS, Some(FIELD)));
    }
    Ok(())
}

/// Why a template use cannot go out as `turn_system`. Not a refusal: the
/// runner falls through to the next mode in the list, and the send's
/// `prompt` goes out at user authority.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateProblem {
    /// The sender's plan entry declares no template with that id.
    UnknownTemplate,
    /// The template's digest is not the one the send names.
    DigestMismatch,
    /// A slot the template declares has no value.
    MissingSlot(String),
    /// A value for a slot the template does not declare.
    UnknownSlot(String),
    /// A value outside its slot's type or range, an unassigned tag, or an
    /// enum value the plan does not list.
    InvalidSlot(String),
}

/// Render `use_` from the templates the plan declares for its sender.
/// `assigned` says whether the session assigned a tag number. The runner
/// renders and validates every slot itself; the sender's word is never
/// enough.
pub fn render(
    templates: &[Template],
    use_: &TemplateUse,
    assigned: impl Fn(u64) -> bool,
) -> Result<String, TemplateProblem> {
    let template = templates
        .iter()
        .find(|template| template.id == use_.id)
        .ok_or(TemplateProblem::UnknownTemplate)?;
    if template_digest(template) != use_.digest {
        return Err(TemplateProblem::DigestMismatch);
    }
    if let Some(extra) = use_
        .slots
        .keys()
        .find(|name| !template.slots.contains_key(*name))
    {
        return Err(TemplateProblem::UnknownSlot(extra.clone()));
    }
    let mut rendered = BTreeMap::new();
    for (name, slot) in &template.slots {
        let value = use_
            .slots
            .get(name)
            .ok_or_else(|| TemplateProblem::MissingSlot(name.clone()))?;
        let invalid = || TemplateProblem::InvalidSlot(name.clone());
        let text = match slot {
            Slot::Integer { min, max } => {
                let n = value
                    .as_i64()
                    .filter(|n| n >= min && n <= max)
                    .ok_or_else(invalid)?;
                n.to_string()
            }
            Slot::Percentage => {
                let n = value.as_u64().filter(|n| *n <= 100).ok_or_else(invalid)?;
                n.to_string()
            }
            Slot::Tag => {
                let tag = value.as_str().ok_or_else(invalid)?;
                let n = crate::tags::parse_tag(tag).ok_or_else(invalid)?;
                if !assigned(n) {
                    return Err(invalid());
                }
                tag.to_owned()
            }
            Slot::Enum { values } => {
                let chosen = value.as_str().ok_or_else(invalid)?;
                if !values.iter().any(|v| v == chosen) {
                    return Err(invalid());
                }
                chosen.to_owned()
            }
        };
        rendered.insert(name.as_str(), text);
    }
    let mut out = String::new();
    let mut rest = template.text.as_str();
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..]
            .find('}')
            .expect("plan_senders checked the text")
            + open;
        out.push_str(&rendered[&rest[open + 1..close]]);
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Whether a send arriving from `sender` with `key` replaces a queued send
/// from `queued_sender` with `queued_key`: only a send still queued and
/// undelivered, from the same sender, with the same key. `send_id` plays no
/// part: a retry with the same `send_id` is the same send.
pub fn replaces(
    sender: &str,
    key: Option<&str>,
    queued_sender: &str,
    queued_key: Option<&str>,
    queued_delivered: bool,
) -> bool {
    !queued_delivered && sender == queued_sender && key.is_some() && key == queued_key
}

/// Whether a send that expires at `expires_at_ms` is dropped at a boundary
/// at `now_ms`: from the expiry time on.
pub fn expired(expires_at_ms: Option<u64>, now_ms: u64) -> bool {
    expires_at_ms.is_some_and(|at| now_ms >= at)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{read::ReadMessage, send::SendReply, send::SendRequest, send::SendState, vectors};
    use serde_json::json;

    fn file() -> Value {
        vectors::load("delivery-order.json")
    }

    fn modes(value: &Value) -> Vec<Mode> {
        serde_json::from_value(value.clone()).unwrap()
    }

    fn declared(case: &Value) -> Vec<&str> {
        case["declares"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect()
    }

    #[test]
    fn legacy_delivery_maps_onto_lists_and_interrupt_has_none() {
        let file = file();
        for case in vectors::cases(&file, "equivalences") {
            let name = case["name"].as_str().unwrap();
            let mut request = json!({"prompt": "p", "send_id": "s"});
            if !case["delivery"].is_null() {
                request["delivery"] = case["delivery"].clone();
            }
            let request: SendRequest = serde_json::from_value(request).unwrap();
            let expected = (!case["modes"].is_null()).then(|| modes(&case["modes"]));
            assert_eq!(legacy_equivalent(request.delivery()), expected, "{name}");
            assert_eq!(request.delivery_order(), Ok(None), "{name}");
        }
    }

    #[test]
    fn sends_decode_their_list_or_refuse_by_field() {
        let file = file();
        for case in vectors::cases(&file, "requests") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &case["request"]);
            let decoded = request.delivery_order().unwrap();
            let expected = (!case["order"].is_null()).then(|| {
                let order = &case["order"];
                let mut expected = DeliveryOrder::new(modes(&order["modes"]));
                expected.dedupe_key = order["dedupe_key"].as_str().map(str::to_owned);
                expected.expires_at_ms = order["expires_at_ms"].as_u64();
                expected.template = order
                    .get("template")
                    .map(|t| serde_json::from_value(t.clone()).unwrap());
                expected
            });
            assert_eq!(decoded, expected, "{name}");
            if let Some(order) = &decoded {
                let rebuilt =
                    SendRequest::new(&request.prompt, &request.send_id).with_delivery_order(order);
                assert_eq!(
                    serde_json::to_value(rebuilt).unwrap(),
                    case["request"],
                    "{name}"
                );
            }
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = serde_json::from_value(case["request"].clone()).unwrap();
            assert_eq!(
                request.delivery_order(),
                Err(case["field"].as_str().unwrap()),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "without_group") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = serde_json::from_value(case["request"].clone()).unwrap();
            match (
                request.check_delivery_order(&declared(case)),
                case.get("refusal"),
            ) {
                (Ok(()), None) => {}
                (Err(field), Some(refusal)) => {
                    assert_eq!(refusal["code"], crate::errors::INVALID_PARAMS, "{name}");
                    assert_eq!(refusal["detail"]["field"], field, "{name}");
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
    }

    fn boundary(value: &Value) -> Boundary {
        let mut boundary = Boundary::default();
        let flag = |key: &str| value[key].as_bool().unwrap_or(false);
        if flag("tool_result") {
            boundary = boundary.with_tool_result();
        }
        if flag("turn_system") {
            boundary = boundary.with_turn_system();
        }
        if flag("run_in_progress") {
            boundary = boundary.with_run_in_progress();
        }
        if flag("idle") {
            boundary = boundary.with_idle();
        }
        boundary
    }

    #[test]
    fn the_first_possible_mode_wins_and_only_idle_start_starts_a_run() {
        let file = file();
        let mut never_started = false;
        for case in vectors::cases(&file, "choices") {
            let name = case["name"].as_str().unwrap();
            let list = modes(&case["modes"]);
            let at = boundary(&case["boundary"]);
            let chosen = choose(&list, &at);
            let expected: Option<Mode> = serde_json::from_value(case["chosen"].clone()).unwrap();
            assert_eq!(chosen, expected, "{name}");
            if chosen == Some(Mode::IdleStart) {
                assert!(DeliveryOrder::new(list.clone()).may_start_a_run(), "{name}");
            }
            if at.idle && !list.contains(&Mode::IdleStart) {
                assert_eq!(
                    chosen, None,
                    "{name}: a list without idle_start started a run"
                );
                never_started = true;
            }
        }
        assert!(
            never_started,
            "no vector shows a list that never starts a run"
        );
    }

    fn problem_of(value: &Value) -> TemplateProblem {
        let slot = || value["slot"].as_str().unwrap().to_owned();
        match value["kind"].as_str().unwrap() {
            "invalid_slot" => TemplateProblem::InvalidSlot(slot()),
            "missing_slot" => TemplateProblem::MissingSlot(slot()),
            "unknown_slot" => TemplateProblem::UnknownSlot(slot()),
            "digest_mismatch" => TemplateProblem::DigestMismatch,
            "unknown_template" => TemplateProblem::UnknownTemplate,
            other => panic!("unknown problem {other}"),
        }
    }

    #[test]
    fn templates_render_only_with_every_typed_slot_valid() {
        let file = file();
        let templates = &file["templates"];
        let sender: PlanSender = vectors::round_trip("template sender", &templates["sender"]);
        let assigned: Vec<u64> =
            serde_json::from_value(templates["assigned_tags"].clone()).unwrap();
        let has = |n: u64| assigned.contains(&n);
        for case in vectors::cases(templates, "uses") {
            let name = case["name"].as_str().unwrap();
            let use_: TemplateUse = vectors::round_trip(name, &case["use"]);
            let outcome = render(&sender.templates, &use_, has);
            match case.get("rendered") {
                Some(text) => assert_eq!(outcome.as_deref(), Ok(text.as_str().unwrap()), "{name}"),
                None => assert_eq!(outcome, Err(problem_of(&case["problem"])), "{name}"),
            }
        }
        // What goes out: the rendering as system when it rendered and the
        // provider supports it, otherwise the send's prompt at user authority.
        for case in vectors::cases(&file, "fall_through") {
            let name = case["name"].as_str().unwrap();
            let rendered = (!case["use"].is_null())
                .then(|| serde_json::from_value::<TemplateUse>(case["use"].clone()).unwrap())
                .and_then(|use_| render(&sender.templates, &use_, has).ok());
            let mut at = Boundary::default().with_run_in_progress();
            if rendered.is_some() && case["provider_supports"] == true {
                at = at.with_turn_system();
            }
            let mode = choose(&modes(&case["modes"]), &at).unwrap();
            let text = if mode == Mode::TurnSystem {
                rendered.unwrap()
            } else {
                "prompt".to_owned()
            };
            assert_eq!(case["delivered"]["mode"], mode.as_str(), "{name}");
            assert_eq!(case["delivered"]["text"], text.as_str(), "{name}");
            assert_eq!(mode.is_user_authority(), mode != Mode::TurnSystem);
        }
    }

    #[test]
    fn plans_name_senders_with_typed_templates() {
        let file = file();
        for case in vectors::cases(&file, "plans") {
            let name = case["name"].as_str().unwrap();
            let plan = case["plan"].as_object().unwrap();
            match (check_plan(plan, &declared(case)), case.get("refusal")) {
                (Ok(()), None) => assert_eq!(
                    plan_senders(plan).unwrap().len() as u64,
                    case["senders"].as_u64().unwrap(),
                    "{name}"
                ),
                (Err(field), Some(refusal)) => {
                    assert_eq!(refusal["code"], crate::errors::INVALID_PARAMS, "{name}");
                    assert_eq!(refusal["detail"]["field"], field, "{name}");
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
    }

    #[test]
    fn admission_matches_the_attested_principal_against_the_plan() {
        let file = file();
        let plan = file["plans"][0]["plan"].as_object().unwrap().clone();
        let senders = plan_senders(&plan).unwrap();
        let mut unnamed_refused = false;
        for case in vectors::cases(&file, "admission") {
            let name = case["name"].as_str().unwrap();
            let outcome = admit(
                case["caller"].as_str().unwrap(),
                case["owner"].as_str(),
                &senders,
                &modes(&case["modes"]),
            );
            match (outcome, case.get("refusal")) {
                (Ok(()), None) => {}
                (Err((code, field)), Some(refusal)) => {
                    assert_eq!(refusal["code"], code, "{name}");
                    assert_eq!(refusal["detail"]["field"].as_str(), field, "{name}");
                    assert!(!crate::errors::is_retryable(code), "{name}");
                    unnamed_refused |= code == crate::errors::SCOPE_OWNER_MISMATCH;
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
        assert!(unnamed_refused);
    }

    #[test]
    fn dedupe_replaces_only_a_queued_send_and_expiry_drops_one() {
        let file = file();
        for case in vectors::cases(&file, "dedupe") {
            let name = case["name"].as_str().unwrap();
            let queued = &case["queued"];
            let arriving = &case["arriving"];
            assert_eq!(
                replaces(
                    arriving["sender"].as_str().unwrap(),
                    arriving["key"].as_str(),
                    queued["sender"].as_str().unwrap(),
                    queued["key"].as_str(),
                    queued["delivered"].as_bool().unwrap(),
                ),
                case["replaces"].as_bool().unwrap(),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "expiry") {
            let name = case["name"].as_str().unwrap();
            assert_eq!(
                expired(
                    case["expires_at_ms"].as_u64(),
                    case["now_ms"].as_u64().unwrap()
                ),
                case["expired"].as_bool().unwrap(),
                "{name}"
            );
        }
        let states: Vec<SendState> = vectors::cases(&file, "replies")
            .iter()
            .map(|case| {
                vectors::round_trip::<SendReply>(case["name"].as_str().unwrap(), &case["reply"])
                    .send_state()
            })
            .collect();
        assert!(states.contains(&SendState::Superseded));
        assert!(states.contains(&SendState::Expired));
    }

    #[test]
    fn delivered_entries_record_their_mode() {
        let file = file();
        let mut seen = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "entries") {
            let name = case["name"].as_str().unwrap();
            let entry: ReadMessage = vectors::round_trip(name, &case["entry"]);
            let delivered = entry.delivered_as.expect("delivered_as");
            assert_eq!(
                delivered.call_key.is_some(),
                delivered.mode == modes::TOOL_RESULT,
                "{name}"
            );
            seen.insert(delivered.mode);
        }
        for mode in [
            modes::TOOL_RESULT,
            modes::TURN_SYSTEM,
            modes::STEER,
            modes::IDLE_START,
        ] {
            assert!(seen.contains(mode), "no entry delivered as {mode}");
        }
    }
}
