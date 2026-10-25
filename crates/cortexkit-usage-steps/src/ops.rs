use serde::{Deserialize, Serialize};

use crate::{codes, Cursor, StepRow};

/// Request of `usage.steps.subscribe`: `{}` for every notice, or
/// `{harness, session}` for the notices of one session. The provider refuses
/// an unknown member `malformed`, and so does this type.
///
/// `harness` and `session` are both optional and also accept `null`; each
/// must be given together with the other or not at all. This type does not
/// check that pairing: the provider refuses a request naming only one
/// `invalid_request`, as it does for `limit`. See
/// [`SubscribeRequest::check_pairing`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SubscribeRequest {
    /// The producer to follow; see [`StepRow::harness`](crate::StepRow::harness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// The session to follow, matched exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
}

impl SubscribeRequest {
    /// Notices for every session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Notices only when a row of this harness and session is committed.
    pub fn for_session(mut self, harness: impl Into<String>, session: impl Into<String>) -> Self {
        self.harness = Some(harness.into());
        self.session = Some(session.into());
        self
    }

    /// Reports a request naming only one of `harness` and `session`. The
    /// pair comes together or not at all; the provider refuses a request that
    /// names one alone `invalid_request`. Both together and neither are fine.
    pub fn check_pairing(&self) -> Result<(), &'static str> {
        pairing::check(self.harness.is_some(), self.session.is_some())
    }
}

/// The `harness`-with-`session` rule both requests share. The request types
/// accept either member alone, as they accept any `limit`. The pairing helper
/// returns an error naming the missing member, and the provider refuses such a
/// request `invalid_request`.
pub(crate) mod pairing {
    pub(crate) fn check(harness: bool, session: bool) -> Result<(), &'static str> {
        match (harness, session) {
            (true, true) | (false, false) => Ok(()),
            (true, false) => Err("request names `harness` without `session`"),
            (false, true) => Err("request names `session` without `harness`"),
        }
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
///
/// `harness` and `session` narrow the read to one session's rows. Without
/// `after`, such a read also starts at the oldest row the feed holds. They
/// come together or not at all: this type accepts either alone, and the
/// provider refuses one alone `invalid_request`, as it refuses a bad
/// `limit`. See [`ReadRequest::check_pairing`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReadRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Cursor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// The producer to read rows of; see [`StepRow::harness`](crate::StepRow::harness).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// The session to read rows of, matched exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
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

    /// Reads only the rows whose `harness` and `session` are these.
    pub fn for_session(mut self, harness: impl Into<String>, session: impl Into<String>) -> Self {
        self.harness = Some(harness.into());
        self.session = Some(session.into());
        self
    }

    /// Reports a request naming only one of `harness` and `session`. The
    /// pair comes together or not at all; the provider refuses a request that
    /// names one alone `invalid_request`. Both together and neither are fine.
    pub fn check_pairing(&self) -> Result<(), &'static str> {
        pairing::check(self.harness.is_some(), self.session.is_some())
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
