//! The `delivery_order` group: an ordered list of delivery modes on
//! `session.send`, the senders a session's plan admits, and the templates
//! whose text may go out with system authority.
//!
//! A runner that declares `delivery_order`
//! ([`crate::capabilities::DELIVERY_ORDER`]) accepts `delivery: [mode, ...]`
//! on a send, the ordered list riding in `delivery` itself
//! ([`DeliveryOrder`]). At each request boundary it
//! takes the first mode in the list that is possible there ([`choose`]); if
//! none is, the send stays queued, durably, for the next boundary. A list
//! without `idle_start` never starts a run.
//!
//! A string `delivery` keeps its meaning and maps onto lists
//! ([`legacy_equivalent`]): `queue` (or absent) is `[idle_start]`, `steer` is
//! `[steer, idle_start]`, and `interrupt` has no list form. The overload
//! fails closed: a runner without the group refuses a list naming `delivery`,
//! as an older runner's strict decode of `delivery` already does, so a list
//! never goes out as a legacy `queue`. The members that ride with a list
//! (`dedupe_key`, `expires_at_ms`, `template`) are valid only beside one.
//!
//! A send has one text whatever mode carries it: its `prompt`, or, for a
//! send naming a plan `template` (which carries no `prompt`), the text the
//! runner renders from that template and the send's typed slots
//! ([`render`]). A template that cannot render refuses the send.
//! `tool_result`, `steer` and `idle_start` deliver at user authority;
//! `turn_system` delivers a turn-scoped system message, possible only for a
//! rendered template, and otherwise the runner falls through to the next
//! mode, carrying the same text at user authority.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{capabilities, errors, send::Delivery};

/// The send member holding the ordered list, `delivery`, and the `field`
/// an `invalid_params` refusal names for a malformed list, a list sent to a
/// runner without the group, or a mode the sender's grant excludes.
pub const FIELD: &str = crate::send::DELIVERY_FIELD;
/// The send member naming a queued send's replacement key.
pub const DEDUPE_KEY_FIELD: &str = "dedupe_key";
/// The send member giving the time after which a queued send is dropped.
pub const EXPIRES_AT_FIELD: &str = "expires_at_ms";
/// The send member naming the plan template a `turn_system` delivery renders.
pub const TEMPLATE_FIELD: &str = "template";
/// The members valid only beside a list in `delivery`.
pub const MEMBERS: &[&str] = &[DEDUPE_KEY_FIELD, EXPIRES_AT_FIELD, TEMPLATE_FIELD];

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

    /// The absent-`delivery_order_modes` mapping from module-level capability
    /// groups. Use [`Self::is_declared_by`] when a describe answer is available.
    /// `delivery_order` declares the list-only modes;
    /// `steer` and `idle_start` reuse the legacy `steer` and `queue` groups.
    pub fn is_declared<S: AsRef<str>>(self, declared: &[S]) -> bool {
        let group = match self {
            Self::ToolResult | Self::TurnSystem => crate::capabilities::DELIVERY_ORDER,
            Self::Steer => crate::capabilities::STEER,
            Self::IdleStart => crate::capabilities::QUEUE,
        };
        declared.iter().any(|name| name.as_ref() == group)
    }

    /// Whether the runner declares this list mode after optional narrowing.
    /// A listed mode never bypasses its capability group. Validate the answer
    /// with [`crate::describe::check_describe`] before relying on it.
    pub fn is_declared_by(self, describe: &crate::describe::RoleDescribe) -> bool {
        self.is_declared(&describe.capabilities)
            && describe.delivery_order_modes.as_ref().is_none_or(|modes| {
                describe.declares(crate::capabilities::DELIVERY_ORDER)
                    && modes.iter().any(|mode| mode == self.as_str())
            })
    }
}

/// The modes' spellings, as a read entry's `delivered_as.mode` names them.
/// That member is an open string: a consumer keeps a mode it does not know.
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

    /// The members that ride beside the list, as set (`dedupe_key`,
    /// `expires_at_ms`, `template`); the list itself rides in `delivery`.
    pub fn to_members(&self) -> Map<String, Value> {
        let mut members = Map::new();
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

    /// The members as a send carries them: the list as an array in
    /// `delivery`, beside [`Self::to_members`].
    pub fn to_send_members(&self) -> Map<String, Value> {
        let mut members = self.to_members();
        members.insert(
            FIELD.into(),
            serde_json::to_value(&self.modes).expect("modes serialize"),
        );
        members
    }

    /// Build the list's send from `modes` (the array a send's `delivery`
    /// carried, already checked by its decode) and the members in `params`.
    /// `Ok(None)` without a list. On a malformed send, the field of the
    /// `invalid_params` refusal: a member without a list is refused naming
    /// that member, as is a malformed member ([`DEDUPE_KEY_FIELD`],
    /// [`EXPIRES_AT_FIELD`], [`TEMPLATE_FIELD`]).
    pub fn decode(
        modes: Option<&[Mode]>,
        params: &Map<String, Value>,
    ) -> Result<Option<Self>, &'static str> {
        let Some(modes) = modes else {
            for member in MEMBERS {
                if params.contains_key(*member) {
                    return Err(member);
                }
            }
            return Ok(None);
        };
        let mut order = Self::new(modes.to_vec());
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
    /// One of [`modes`]; a mode this crate does not know decodes as itself.
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
    /// A part of the session, named by its host identity `{mid, part}`
    /// (`delivery_order::PartRef`), never by a number. The runner renders
    /// the number the request carrying the send shows for that part
    /// (`§N§`); a part that request does not show ends the send
    /// `template_part_gone`. Spelled `part_tag`: a template that declared a
    /// `tag` slot before llm-runner 0.4.0 no longer decodes, and its digest
    /// changes with the new spelling.
    PartTag,
    /// Exactly one of the strings in `values`.
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
    /// Whether this sender may send the `interrupt` delivery string, which
    /// cancels the running turn and starts its message, as the owner's does.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub interrupt: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub templates: Vec<Template>,
}

impl PlanSender {
    pub fn new(module: impl Into<String>) -> Self {
        Self {
            module: module.into(),
            idle_start: false,
            interrupt: false,
            templates: Vec::new(),
        }
    }
    pub fn with_idle_start(mut self) -> Self {
        self.idle_start = true;
        self
    }
    /// Grant this sender the `interrupt` delivery string.
    pub fn with_interrupt(mut self) -> Self {
        self.interrupt = true;
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

/// A string `delivery`'s admission on a runner that declares
/// `delivery_order`, with the arguments of [`admit`]. `queue` and `steer`
/// stand for their lists ([`legacy_equivalent`]) and are admitted exactly as
/// those lists are, so a named sender granted `idle_start` may send either
/// string. `interrupt` has no list form: a named sender may send it only
/// when its entry grants `interrupt`, and is otherwise refused
/// `invalid_params` naming [`FIELD`]. The owner is admitted with any string.
///
/// Admission decides only who may send. A run the send starts or joins
/// runs under the session owner's authority (its principal and scope),
/// never the sender's.
pub fn admit_delivery(
    caller: &str,
    owner: Option<&str>,
    senders: &[PlanSender],
    delivery: Delivery,
) -> Result<(), (&'static str, Option<&'static str>)> {
    if let Some(modes) = legacy_equivalent(delivery) {
        return admit(caller, owner, senders, &modes);
    }
    if owner.is_none_or(|owner| owner == caller) {
        return Ok(());
    }
    let sender = senders
        .iter()
        .find(|sender| sender.module == caller)
        .ok_or((errors::SCOPE_OWNER_MISMATCH, None))?;
    if !sender.interrupt {
        return Err((errors::INVALID_PARAMS, Some(FIELD)));
    }
    Ok(())
}

/// The value of a `part_tag` slot: the part's host identity, the `mid` of
/// its message and its `part` key as `session.read` names them. Strict: an
/// unknown member, or a value that is not this object (a number, a `§N§`
/// string, free text), is not a value of the slot.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PartRef {
    pub mid: String,
    pub part: String,
}

impl PartRef {
    pub fn new(mid: impl Into<String>, part: impl Into<String>) -> Self {
        Self {
            mid: mid.into(),
            part: part.into(),
        }
    }
}

/// Why a send's template cannot render. The send has no text, so it is
/// never delivered and never falls through. Every problem but
/// [`TemplateProblem::PartGone`] is refused at admission `invalid_params
/// {field: "template"}`, writing nothing. `PartGone` is refused
/// `template_part_gone {slot}` at admission, and ends a queued send in the
/// terminal send state `template_part_gone` when its part goes before the
/// send is delivered.
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
    /// A value outside its slot's type or range, a `part_tag` value that is
    /// not `{mid, part}`, or an enum value the plan does not list.
    InvalidSlot(String),
    /// A `part_tag` slot names a part the carrying request does not show.
    PartGone(String),
}

impl TemplateProblem {
    /// The refusal of a send whose template has this problem: the code and
    /// its detail.
    pub fn refusal(&self) -> (&'static str, Value) {
        match self {
            Self::PartGone(slot) => (
                errors::TEMPLATE_PART_GONE,
                serde_json::json!({ "slot": slot }),
            ),
            _ => (
                errors::INVALID_PARAMS,
                serde_json::json!({ "field": TEMPLATE_FIELD }),
            ),
        }
    }
}

/// Render `use_` from the templates the plan declares for its sender: the
/// send's one text, used for whichever mode carries it. `shown` answers the
/// number the request carrying the send shows for a part, or `None` when
/// that request does not show it. The runner renders and validates every
/// slot itself; the sender's word is never enough. A value outside its
/// slot's type, range or listed values is not a value of that slot, so it
/// is as unrenderable as a missing one.
///
/// At admission the runner renders against the request it would make now;
/// at delivery, against the request that carries the send, whose text is
/// then stored and resent byte for byte.
pub fn render(
    templates: &[Template],
    use_: &TemplateUse,
    shown: impl Fn(&PartRef) -> Option<u64>,
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
            Slot::PartTag => {
                let part: PartRef = serde_json::from_value(value.clone()).map_err(|_| invalid())?;
                let n = shown(&part).ok_or_else(|| TemplateProblem::PartGone(name.clone()))?;
                crate::tags::format_tag(n).ok_or_else(invalid)?
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
    fn list_modes_follow_their_individual_capability_declarations() {
        let list_group = [crate::capabilities::DELIVERY_ORDER];
        assert!(Mode::ToolResult.is_declared(&list_group));
        assert!(Mode::TurnSystem.is_declared(&list_group));
        assert!(!Mode::Steer.is_declared(&list_group));
        assert!(!Mode::IdleStart.is_declared(&list_group));

        assert!(Mode::Steer.is_declared(&[crate::capabilities::STEER]));
        assert!(Mode::IdleStart.is_declared(&[crate::capabilities::QUEUE]));
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
                let rebuilt = if order.template.is_some() {
                    SendRequest::template(&request.send_id, order)
                } else {
                    SendRequest::new(request.prompt_text(), &request.send_id)
                        .with_delivery_order(order)
                };
                assert_eq!(
                    serde_json::to_value(rebuilt).unwrap(),
                    case["request"],
                    "{name}"
                );
            }
        }
        // A malformed `delivery` does not decode, and the error names it.
        for case in vectors::cases(&file, "undecodable") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error =
                serde_json::from_value::<SendRequest>(case["request"].clone()).expect_err(name);
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        // A member without a list, or a malformed member, decodes and is
        // refused naming it.
        let mut without_list = false;
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = serde_json::from_value(case["request"].clone()).unwrap();
            assert_eq!(
                request.delivery_order(),
                Err(case["field"].as_str().unwrap()),
                "{name}"
            );
            without_list |= request.delivery_order.is_none();
        }
        assert!(
            without_list,
            "no vector refuses a member sent without a list"
        );
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
        for case in vectors::cases(&file, "undeclared_mode_lists") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &case["request"]);
            let order = request.delivery_order().unwrap().expect("a list");
            let declared = declared(case);
            assert_eq!(request.check_delivery_order(&declared), Ok(()), "{name}");
            assert!(
                !order.modes.iter().any(|mode| mode.is_declared(&declared)),
                "{name} has a declared mode"
            );
            assert_eq!(
                case["refusal"]["code"],
                crate::errors::DELIVERY_UNSUPPORTED,
                "{name}"
            );
            assert_eq!(
                crate::send::delivery_unsupported_detail(&order.modes),
                case["refusal"]["detail"],
                "{name}"
            );
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
            "part_gone" => TemplateProblem::PartGone(slot()),
            other => panic!("unknown problem {other}"),
        }
    }

    #[test]
    fn templates_render_only_with_every_typed_slot_valid() {
        let file = file();
        let templates = &file["templates"];
        let sender: PlanSender = vectors::round_trip("template sender", &templates["sender"]);
        let shown_parts: Vec<(PartRef, u64)> = vectors::cases(templates, "shown_parts")
            .iter()
            .map(|row| {
                (
                    PartRef::new(row["mid"].as_str().unwrap(), row["part"].as_str().unwrap()),
                    row["number"].as_u64().unwrap(),
                )
            })
            .collect();
        let has = |part: &PartRef| {
            shown_parts
                .iter()
                .find(|(shown, _)| shown == part)
                .map(|(_, number)| *number)
        };
        for case in vectors::cases(templates, "uses") {
            let name = case["name"].as_str().unwrap();
            let use_: TemplateUse = vectors::round_trip(name, &case["use"]);
            let outcome = render(&sender.templates, &use_, has);
            match case.get("rendered") {
                Some(text) => assert_eq!(outcome.as_deref(), Ok(text.as_str().unwrap()), "{name}"),
                None => assert_eq!(outcome, Err(problem_of(&case["problem"])), "{name}"),
            }
        }
        // A send has one text: the template's rendering, or its prompt
        // without a template. A template that cannot render refuses the
        // send; one that renders goes out as system where the provider
        // supports it, otherwise as the same text at user authority.
        for case in vectors::cases(&file, "fall_through") {
            let name = case["name"].as_str().unwrap();
            let rendered = (!case["use"].is_null()).then(|| {
                let use_: TemplateUse = serde_json::from_value(case["use"].clone()).unwrap();
                render(&sender.templates, &use_, has)
            });
            if let Some(refusal) = case.get("refusal") {
                let Some(Err(problem)) = rendered else {
                    panic!("{name}: rendered {rendered:?}");
                };
                let (code, detail) = problem.refusal();
                assert_eq!(refusal["code"], code, "{name}");
                assert_eq!(refusal["detail"], detail, "{name}");
                assert!(crate::errors::CODES.contains(&code), "{name}");
                assert!(!crate::errors::is_retryable(code), "{name}");
                continue;
            }
            let rendered = rendered.map(Result::unwrap);
            let mut at = Boundary::default().with_run_in_progress();
            if rendered.is_some() && case["provider_supports"] == true {
                at = at.with_turn_system();
            }
            let mode = choose(&modes(&case["modes"]), &at).unwrap();
            let text = rendered.unwrap_or_else(|| "prompt".to_owned());
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
    fn string_deliveries_follow_the_sender_grants() {
        let file = file();
        let plan = file["plans"][0]["plan"].as_object().unwrap().clone();
        let senders = plan_senders(&plan).unwrap();
        let granted = senders
            .iter()
            .find(|sender| sender.interrupt)
            .expect("a vector sender holds the interrupt grant");
        let rebuilt = PlanSender::new(&granted.module)
            .with_idle_start()
            .with_interrupt();
        assert_eq!(&rebuilt, granted);
        let mut refused_interrupt = false;
        let mut admitted_named_queue = false;
        for case in vectors::cases(&file, "string_admission") {
            let name = case["name"].as_str().unwrap();
            let caller = case["caller"].as_str().unwrap();
            let owner = case["owner"].as_str();
            let request: SendRequest = vectors::round_trip(name, &case["request"]);
            let delivery = request.delivery();
            match (
                admit_delivery(caller, owner, &senders, delivery),
                case.get("refusal"),
            ) {
                (Ok(()), None) => {
                    admitted_named_queue |= delivery == Delivery::Queue && Some(caller) != owner;
                }
                (Err((code, field)), Some(refusal)) => {
                    assert_eq!(refusal["code"], code, "{name}");
                    assert_eq!(refusal["detail"]["field"].as_str(), field, "{name}");
                    assert!(!crate::errors::is_retryable(code), "{name}");
                    refused_interrupt |=
                        delivery == Delivery::Interrupt && code == crate::errors::INVALID_PARAMS;
                }
                (outcome, refusal) => panic!("{name}: {outcome:?} against {refusal:?}"),
            }
        }
        assert!(
            refused_interrupt,
            "no vector refuses an ungranted interrupt"
        );
        assert!(
            admitted_named_queue,
            "no vector admits a named sender's string queue"
        );
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
        assert!(states.contains(&SendState::TemplatePartGone));
        let gone = vectors::cases(&file, "replies")
            .iter()
            .find(|case| case["reply"]["state"] == crate::send::states::TEMPLATE_PART_GONE)
            .map(|case| serde_json::from_value::<SendReply>(case["reply"].clone()).unwrap())
            .unwrap();
        assert_eq!(gone.slot.as_deref(), Some("part"));
    }

    /// A `part_tag` slot changes the template's bytes, so its digest, and an
    /// old `tag` slot no longer decodes: a sender holding a pre-0.4.0
    /// template is refused loudly rather than rendering a stale number.
    #[test]
    fn a_part_tag_slot_replaces_the_tag_slot() {
        let slot: Slot = serde_json::from_value(serde_json::json!({"type": "part_tag"})).unwrap();
        assert_eq!(slot, Slot::PartTag);
        assert!(serde_json::from_value::<Slot>(serde_json::json!({"type": "tag"})).is_err());
        assert!(serde_json::from_value::<PartRef>(
            serde_json::json!({"mid": "m", "part": "0", "tag": "§1§"})
        )
        .is_err());
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
