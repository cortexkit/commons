//! The applied plan's system text and offered tools, not a provider-request copy.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A whole-plan read. The management route binds the session, as for `session.read`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct PlanViewRequest {}

impl PlanViewRequest {
    pub fn new() -> Self {
        Self::default()
    }
}

/// A closed `session.plan_view` reply. Decoding validates item ranges,
/// a newer pending generation and its notes, and the required `as_planned: true` marker.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(try_from = "WirePlanView")]
#[non_exhaustive]
pub struct PlanView {
    pub generation: u64,
    /// Omitted when no application time was recorded, never synthesized.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub applied_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest_step: Option<NewestStep>,
    pub system: System,
    /// The order in which the model is offered the tools.
    pub tools: Vec<Tool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<Pending>,
    /// Always true: a login plugin or gateway may prepend other text later.
    pub as_planned: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WirePlanView {
    generation: u64,
    #[serde(default)]
    applied_at_ms: Option<u64>,
    #[serde(default)]
    newest_step: Option<NewestStep>,
    system: System,
    tools: Vec<Tool>,
    #[serde(default)]
    pending: Option<Pending>,
    #[serde(deserialize_with = "deserialize_as_planned")]
    as_planned: bool,
}

fn deserialize_as_planned<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match bool::deserialize(deserializer) {
        Ok(true) => Ok(true),
        _ => Err(serde::de::Error::custom("as_planned must be true")),
    }
}

impl TryFrom<WirePlanView> for PlanView {
    type Error = &'static str;

    fn try_from(wire: WirePlanView) -> Result<Self, Self::Error> {
        let view = Self {
            generation: wire.generation,
            applied_at_ms: wire.applied_at_ms,
            newest_step: wire.newest_step,
            system: wire.system,
            tools: wire.tools,
            pending: wire.pending,
            as_planned: wire.as_planned,
        };
        view.check()?;
        Ok(view)
    }
}

impl PlanView {
    pub fn new(generation: u64, system: System, tools: Vec<Tool>) -> Self {
        Self {
            generation,
            applied_at_ms: None,
            newest_step: None,
            system,
            tools,
            pending: None,
            as_planned: true,
        }
    }

    pub fn with_applied_at_ms(mut self, at_ms: u64) -> Self {
        self.applied_at_ms = Some(at_ms);
        self
    }

    pub fn with_newest_step(mut self, step: NewestStep) -> Self {
        self.newest_step = Some(step);
        self
    }

    pub fn with_pending(mut self, pending: Pending) -> Self {
        self.pending = Some(pending);
        self
    }

    /// Check a constructed reply before serving it; errors name the invalid member.
    pub fn check(&self) -> Result<(), &'static str> {
        if !self.as_planned {
            return Err("as_planned");
        }
        self.system.check()?;
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.generation <= self.generation)
        {
            return Err("pending.generation");
        }
        if let Some(pending) = &self.pending {
            pending.check()?;
        }
        Ok(())
    }
}

/// Newest request with a recorded send time under the applied generation.
/// Absent from the view when none exists; not a saved request body.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct NewestStep {
    pub at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_mid: Option<String>,
}

impl NewestStep {
    pub fn new(at_ms: u64) -> Self {
        Self {
            at_ms,
            reply_mid: None,
        }
    }

    pub fn with_reply_mid(mut self, mid: impl Into<String>) -> Self {
        self.reply_mid = Some(mid.into());
        self
    }
}

/// Exact system text built by the plan, with optional provider-item byte ranges.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct System {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Vec<SystemItem>>,
}

impl System {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            items: None,
        }
    }

    pub fn with_items(mut self, items: Vec<SystemItem>) -> Self {
        self.items = Some(items);
        self
    }

    pub fn check(&self) -> Result<(), &'static str> {
        let mut previous_end = 0;
        for item in self.items.iter().flatten() {
            let (Ok(start), Ok(end)) = (usize::try_from(item.start), usize::try_from(item.end))
            else {
                return Err("system.items");
            };
            if start < previous_end
                || start >= end
                || end > self.text.len()
                || !self.text.is_char_boundary(start)
                || !self.text.is_char_boundary(end)
            {
                return Err("system.items");
            }
            previous_end = end;
        }
        Ok(())
    }
}

/// One nonempty provider item. Text used to join provider items, and providers
/// with no text, have no range.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SystemItem {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    pub start: u64,
    pub end: u64,
}

impl SystemItem {
    pub fn new(provider: impl Into<String>, start: u64, end: u64) -> Self {
        Self {
            provider: provider.into(),
            preset: None,
            start,
            end,
        }
    }

    pub fn with_preset(mut self, preset: impl Into<String>) -> Self {
        self.preset = Some(preset.into());
        self
    }
}

/// A tool as offered by the plan; the schema remains an arbitrary JSON value.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Tool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<ToolTokens>,
}

impl Tool {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema,
            provider: provider.into(),
            tokens: None,
        }
    }

    pub fn with_tokens(mut self, tokens: ToolTokens) -> Self {
        self.tokens = Some(tokens);
        self
    }
}

/// Per-tool token count. `exact: true` means the provider's own count, reported
/// by its count endpoint or usage. A local tokenizer's count is `exact: false`,
/// even when it is close.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ToolTokens {
    pub count: u64,
    pub exact: bool,
}

impl ToolTokens {
    pub fn estimate(count: u64) -> Self {
        Self {
            count,
            exact: false,
        }
    }

    pub fn measured(count: u64) -> Self {
        Self { count, exact: true }
    }
}

/// A newer plan not yet applied in full. The model may not have been told about it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Pending {
    pub generation: u64,
    /// Exact texts delivered to the model, in delivery order; absent if none were sent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_notes"
    )]
    pub notes: Option<Vec<String>>,
}

fn check_notes(notes: &[String]) -> Result<(), &'static str> {
    if notes.is_empty() || notes.iter().any(String::is_empty) {
        return Err("pending.notes");
    }
    Ok(())
}

fn deserialize_notes<'de, D>(deserializer: D) -> Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let notes = Vec::<String>::deserialize(deserializer)?;
    check_notes(&notes).map_err(serde::de::Error::custom)?;
    Ok(Some(notes))
}

impl Pending {
    pub fn new(generation: u64) -> Self {
        Self {
            generation,
            notes: None,
        }
    }

    pub fn with_notes(mut self, notes: Vec<String>) -> Result<Self, &'static str> {
        check_notes(&notes)?;
        self.notes = Some(notes);
        Ok(self)
    }

    pub fn check(&self) -> Result<(), &'static str> {
        if let Some(notes) = &self.notes {
            check_notes(notes)?;
        }
        Ok(())
    }
}
