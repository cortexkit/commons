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
#[serde(try_from = "AnswerWire")]
#[non_exhaustive]
pub struct TagReadAnswer {
    pub entry: ReadMessage,
    pub part: u32,
    /// The resolved part is host-written, never a citation of the person's words.
    /// Emit only after citation readers support it; false is absent on the wire.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub host: bool,
    /// Open presentation code for the resolved host part, never a text rewrite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_code: Option<String>,
}

#[derive(Deserialize)]
struct AnswerWire {
    entry: ReadMessage,
    part: u32,
    #[serde(default)]
    host: bool,
    #[serde(default, deserialize_with = "crate::display::host_codes::deserialize")]
    host_code: Option<String>,
}

impl TryFrom<AnswerWire> for TagReadAnswer {
    type Error = &'static str;

    fn try_from(wire: AnswerWire) -> Result<Self, Self::Error> {
        crate::display::host_codes::check(wire.host, wire.host_code.as_deref())?;
        Ok(Self {
            entry: wire.entry,
            part: wire.part,
            host: wire.host,
            host_code: wire.host_code,
        })
    }
}

impl TagReadAnswer {
    pub fn new(entry: ReadMessage, part: u32) -> Self {
        Self {
            entry,
            part,
            host: false,
            host_code: None,
        }
    }

    /// Record the resolved part's host origin from the same exact knowledge
    /// used for display parts. Native `synthetic` rows are host-written too.
    pub fn with_host(mut self, host: bool) -> Self {
        self.host = host;
        if !host {
            self.host_code = None;
        }
        self
    }

    /// Set both `host` and its presentation code from the same exact match.
    /// Native `synthetic` rows carry no code. Unknown codes fall back to text.
    pub fn with_host_code(mut self, code: impl Into<String>) -> Self {
        self.host = true;
        self.host_code = Some(code.into());
        self
    }
}
