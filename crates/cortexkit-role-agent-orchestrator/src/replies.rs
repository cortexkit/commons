//! Success bodies. Unknown members are ignored, and new states are preserved.

use serde::{Deserialize, Serialize};

open_string!(ItemState {
    Open => "open",
    Done => "done",
    Failed => "failed",
    Cancelled => "cancelled",
    Lost => "lost",
    SessionEnded => "session_ended",
});

impl ItemState {
    /// Only `open` means the work is still running.
    pub fn is_ended(&self) -> bool {
        self.as_str() != "open"
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Item {
    pub key: String,
    pub label: String,
    pub state: ItemState,
    pub opened_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ms: Option<u64>,
}

impl Item {
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        state: ItemState,
        opened_at_ms: u64,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            state,
            opened_at_ms,
            ended_at_ms: None,
        }
    }

    pub fn with_ended_at_ms(mut self, ended_at_ms: u64) -> Self {
        self.ended_at_ms = Some(ended_at_ms);
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ItemReply {
    pub item: Item,
}

impl ItemReply {
    pub fn new(item: Item) -> Self {
        Self { item }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SyncReply {
    pub lost: Vec<String>,
    pub unknown: Vec<String>,
}

impl SyncReply {
    pub fn new(lost: Vec<String>, unknown: Vec<String>) -> Self {
        Self { lost, unknown }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ListReply {
    pub items: Vec<Item>,
}

impl ListReply {
    pub fn new(items: Vec<Item>) -> Self {
        Self { items }
    }
}
