use serde::{Deserialize, Serialize};

/// An opaque feed position issued by the provider.
///
/// Cursors are ordered by the provider's commits, are stable across restarts
/// and are never reused, but apps never parse them or compare their order:
/// they only pass one back as a read's `after`. This type therefore has no
/// ordering; equality only says two values are the same cursor string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Cursor(String);

impl Cursor {
    /// Wraps a cursor string exactly as received.
    pub fn new(cursor: impl Into<String>) -> Self {
        Self(cursor.into())
    }

    /// The cursor string, to send back unchanged.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Cursor {
    fn from(cursor: String) -> Self {
        Self(cursor)
    }
}

impl From<&str> for Cursor {
    fn from(cursor: &str) -> Self {
        Self(cursor.to_owned())
    }
}
