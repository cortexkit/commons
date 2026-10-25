use serde::{Deserialize, Serialize};

use crate::{codes, Cursor, StepRow};

/// Request of `usage.steps.subscribe`: always `{}`. The provider refuses any
/// member `malformed`, and so does this type.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SubscribeRequest {}

impl SubscribeRequest {
    pub fn new() -> Self {
        Self {}
    }
}

/// One StreamData frame of an open subscription: new steps may exist up to
/// `through`, the newest committed cursor.
///
/// Notices are coalesced (at most four a second), and one may cover many
/// steps or none the caller may see. It carries no rows; read them with
/// `usage.steps.read`. Unknown members are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Notice {
    pub through: Cursor,
}

impl Notice {
    pub fn new(through: impl Into<Cursor>) -> Self {
        Self {
            through: through.into(),
        }
    }

    /// Reports a member outside the published v1 notice list, by name. For
    /// the provider's own tests; readers ignore unknown members instead.
    pub fn check_closed(value: &serde_json::Value) -> Result<(), crate::NotClosed> {
        crate::row::check_members("notice", value, &["through"])
    }
}

/// Request of `usage.steps.read`. Both members are optional and also accept
/// `null`; any other member is refused `malformed`, and so does this type.
///
/// Without `after`, the read starts at the oldest row the feed still holds
/// (the rows committed in the last 24 hours). `limit` defaults to 200; 0 is
/// refused `invalid_request` and above 1,000 `range_too_large`. This type
/// does not check `limit`: the provider does.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

impl ReadRequest {
    /// A read from the oldest row the feed holds, with the default limit.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads the rows after `cursor`.
    pub fn after(mut self, cursor: impl Into<Cursor>) -> Self {
        self.after = Some(cursor.into());
        self
    }

    /// Asks for at most `limit` rows.
    pub fn with_limit(mut self, limit: u32) -> Self {
        self.limit = Some(limit);
        self
    }
}

/// Reply of `usage.steps.read`. Unknown members are ignored.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReadReply {
    /// Oldest first. Several rows may share an identity; the last one wins.
    pub rows: Vec<StepRow>,
    /// The cursor to pass as `after` for the following page, or null when
    /// this reply reached `through`. Always sent, as null when absent.
    #[serde(default)]
    pub next: Option<Cursor>,
    /// The newest committed cursor.
    pub through: Cursor,
}

impl ReadReply {
    pub fn new(rows: Vec<StepRow>, next: Option<Cursor>, through: impl Into<Cursor>) -> Self {
        Self {
            rows,
            next,
            through: through.into(),
        }
    }

    /// The cursor the next read passes as `after`: `next` while more pages
    /// remain, otherwise `through`.
    pub fn resume_after(&self) -> &Cursor {
        self.next.as_ref().unwrap_or(&self.through)
    }

    /// Reports a reply member, or a member of any row, outside the published
    /// v1 lists. For the provider's own tests: whatever it emits must stay
    /// within the published lists. Readers ignore unknown members instead.
    pub fn check_closed(value: &serde_json::Value) -> Result<(), crate::NotClosed> {
        crate::row::check_members("reply", value, &["rows", "next", "through"])?;
        if let Some(serde_json::Value::Array(rows)) = value.get("rows") {
            for row in rows {
                StepRow::check_closed(row)?;
            }
        }
        Ok(())
    }
}

/// The body of an Error frame: a refusal of either operation, or the `lagged`
/// or `cancelled` ending of a subscription.
///
/// `code` is one of [`codes`] (an unknown code is kept as received);
/// `message` is for people and is not a stable interface. `lagged` and
/// `cursor_expired` carry `detail` `{"through": <cursor>}`. Unknown members
/// are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Refusal {
    pub code: String,
    #[serde(default)]
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl Refusal {
    /// A refusal without detail.
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }

    /// The `lagged` ending: `through` is the last notified cursor.
    pub fn lagged(through: impl Into<Cursor>, message: impl Into<String>) -> Self {
        Self::new(codes::LAGGED, message).with_through(through)
    }

    /// The `cancelled` ending a provider may send after the caller's own
    /// Cancel. A reader treats it, after its own Cancel, as the normal end.
    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(codes::CANCELLED, message)
    }

    /// The `cursor_expired` refusal of a read: `through` is the newest cursor.
    pub fn cursor_expired(through: impl Into<Cursor>, message: impl Into<String>) -> Self {
        Self::new(codes::CURSOR_EXPIRED, message).with_through(through)
    }

    fn with_through(mut self, through: impl Into<Cursor>) -> Self {
        self.detail = Some(serde_json::json!({ "through": through.into() }));
        self
    }

    /// The cursor in `detail.through`, when the refusal carries one.
    pub fn through(&self) -> Option<Cursor> {
        self.detail
            .as_ref()?
            .get("through")?
            .as_str()
            .map(Cursor::new)
    }
}
