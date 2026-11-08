//! Resolve a model-visible part number in the request that produced a call.
//!
//! The call fixes the meaning: present-day numbering may name another message
//! after an undo. Runners retain the request's mapping, not a current tag index.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{errors, read::ReadMessage, scope_admission::ScopeKind};

/// A flat `session.read` selector, parsed instead of a page request when `tag`
/// is present. Display facts are always included; no other members are legal.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct TagReadRequest {
    pub tag: u64,
    pub call_key: String,
}

impl TagReadRequest {
    pub fn new(tag: u64, call_key: impl Into<String>) -> Self {
        Self {
            tag,
            call_key: call_key.into(),
        }
    }

    /// Decode a selector, returning the exact `invalid_params` field on error.
    /// Any other member, including paging and formatting flags, is refused.
    pub fn decode(params: &Value) -> Result<Self, String> {
        let object = params.as_object().ok_or("tag")?;
        if let Some(field) = object
            .keys()
            .find(|key| *key != "tag" && *key != "call_key")
        {
            return Err(field.clone());
        }
        let tag = object.get("tag").and_then(Value::as_u64).ok_or("tag")?;
        let call_key = object
            .get("call_key")
            .and_then(Value::as_str)
            .ok_or("call_key")?;
        Ok(Self::new(tag, call_key))
    }

    /// A runner without `tag_read` refuses the selector naming `tag`.
    pub fn check_declared(&self, declared: bool) -> Result<(), &'static str> {
        declared.then_some(()).ok_or("tag")
    }

    /// Only the session owner's plain session route may resolve a tag. All
    /// identities and the scope kind come from the daemon, never this request.
    /// A listed reader, flow route or run route gains no owner authority.
    pub fn check_owner(
        &self,
        scope_owner: Option<&str>,
        attested_sender: &str,
        kind: ScopeKind<'_>,
    ) -> Result<(), &'static str> {
        if matches!(kind, ScopeKind::Plain) && scope_owner == Some(attested_sender) {
            Ok(())
        } else {
            Err(errors::SCOPE_OWNER_MISMATCH)
        }
    }
}

/// The entry as shown in the call's model request, with ordinary read display
/// facts, and the zero-based stored top-level part index the tag names.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct TagReadAnswer {
    pub entry: ReadMessage,
    pub part: u32,
}

impl TagReadAnswer {
    pub fn new(entry: ReadMessage, part: u32) -> Self {
        Self { entry, part }
    }
}
