//! Per-argument dispatch routing for plan tool items (CONTRACT §9.25).
//! Admission runs before writes; rendering may run once at admission or on each
//! request. These helpers do not send calls or own durable dispatch intents.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// One argument value and the provider it selects on the agent's own route.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RouteValue {
    pub value: String,
    pub provider: String,
}

impl RouteValue {
    pub fn new(value: impl Into<String>, provider: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            provider: provider.into(),
        }
    }
}

/// The optional `routes` member of one plan tool item.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolRoutes {
    pub argument: String,
    pub description: String,
    pub values: Vec<RouteValue>,
}

/// Routed-only display facts, alongside the existing `dispatched_to` string.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ToolRoute {
    pub argument: String,
    pub value: String,
}

impl ToolRoute {
    pub fn new(argument: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            argument: argument.into(),
            value: value.into(),
        }
    }
}

impl ToolRoutes {
    pub fn new(
        argument: impl Into<String>,
        description: impl Into<String>,
        values: Vec<RouteValue>,
    ) -> Self {
        Self {
            argument: argument.into(),
            description: description.into(),
            values,
        }
    }

    /// Shape only; the refusal names the whole indexed `routes` member.
    pub fn valid(&self) -> bool {
        let bytes = self.argument.as_bytes();
        let mut values = BTreeSet::new();
        (1..=32).contains(&bytes.len())
            && bytes[0].is_ascii_lowercase()
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'_')
            && !self.description.is_empty()
            && self.description.len() <= 512
            && (1..=32).contains(&self.values.len())
            && self.values.iter().all(|entry| {
                !entry.value.is_empty()
                    && entry.value.len() <= 64
                    && values.insert(&entry.value)
                    && valid_provider(&entry.provider)
            })
    }

    /// Check all offered schemas before rendering any of them. An existing
    /// property is a clash even when it has the same schema we would add.
    pub fn check_schema(&self, schema: &Value, index: usize) -> Result<(), String> {
        if schema
            .get("properties")
            .and_then(|properties| properties.get(&self.argument))
            .is_some()
        {
            return Err(field(index));
        }
        Ok(())
    }

    /// Add an optional typed enum. Never add the routing argument to `required`.
    /// Call only after shape and clash checks; the ordinary tool-schema checks
    /// ensure an object input schema with object properties.
    pub fn offer(&self, schema: &Value) -> Value {
        let mut schema = schema.clone();
        if schema.get("properties").is_none() {
            schema["properties"] = json!({});
        }
        schema["properties"][&self.argument] = json!({
            "type": "string", "enum": self.values.iter().map(|entry| &entry.value).collect::<Vec<_>>(),
            "description": self.description
        });
        schema
    }

    /// Resolve the chosen provider and strip only the routing argument.
    /// An error is a call error result before dispatch, not a plan refusal.
    /// The transcript retains the original `arguments`; persist this return
    /// value and target in the dispatch intent before sending at most once.
    pub fn dispatch(
        &self,
        default: &str,
        arguments: &Value,
    ) -> Result<(String, Value, Option<ToolRoute>), &'static str> {
        let mut stripped = arguments.clone();
        let Some(value) = arguments.get(&self.argument) else {
            return Ok((default.to_owned(), stripped, None));
        };
        let value = value.as_str().ok_or("invalid routing value")?;
        let target = self
            .values
            .iter()
            .find(|entry| entry.value == value)
            .ok_or("invalid routing value")?;
        if let Some(object) = stripped.as_object_mut() {
            object.remove(&self.argument);
        }
        Ok((
            target.provider.clone(),
            stripped,
            Some(ToolRoute::new(&self.argument, value)),
        ))
    }
}

fn valid_provider(provider: &str) -> bool {
    // A module id is opaque, including federated ids with colons; route labels
    // are non-empty and contain no whitespace or control characters.
    !provider.is_empty()
        && !provider
            .chars()
            .any(|c| c.is_whitespace() || c.is_control())
}

pub fn field(index: usize) -> String {
    format!("plan.tool_items[{index}].routes")
}

/// Validate declaration and shape for each present member before fetching items
/// or writing anything, even when the item is optional.
pub fn check_plan(plan: &Value, declared: bool) -> Result<(), String> {
    if let Some(items) = plan.get("tool_items").and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            if let Some(value) = item.get("routes") {
                let routes = serde_json::from_value::<ToolRoutes>(value.clone());
                if !declared || !routes.is_ok_and(|routes| routes.valid()) {
                    return Err(field(index));
                }
            }
        }
    }
    Ok(())
}
