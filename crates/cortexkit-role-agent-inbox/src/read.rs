//! Agent-facing pull tools. The route binds the agent's own inbox; no request
//! can select another agent. Reading does not acknowledge a push landing.

use serde::{Deserialize, Serialize};
use serde_json::Value;

open_string!(ItemState {
    Pending => "pending", InFlight => "in_flight", Landed => "landed",
    Unconfirmed => "unconfirmed", Retired => "retired",
});

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListRequest {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReadRequest {
    pub item_id: String,
}

impl ReadRequest {
    pub fn new(item_id: impl Into<String>) -> Self {
        Self {
            item_id: item_id.into(),
        }
    }
}

/// Rendered body includes the implementer's per-item envelope and attested
/// attribution. `data` is opaque, not a trusted attribution source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Item {
    pub item_id: String,
    pub state: ItemState,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl Item {
    pub fn new(item_id: impl Into<String>, state: ItemState, body: impl Into<String>) -> Self {
        Self {
            item_id: item_id.into(),
            state,
            body: body.into(),
            data: None,
        }
    }
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListReply {
    pub items: Vec<Item>,
}

impl ListReply {
    pub fn new(items: Vec<Item>) -> Self {
        Self { items }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReadReply {
    pub item: Option<Item>,
}

impl ReadReply {
    pub fn new(item: Option<Item>) -> Self {
        Self { item }
    }
}
