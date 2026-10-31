//! Optional display views, fetched by the display server off the write path.
//! For a pin, a view's record schema and reads never change; its template can
//! be replaced with a higher `template_revision` (CONTRACT §2.1).

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use subc_protocol::ErrorBody;

pub use cortexkit_role_tool_display::{FieldKind, RecordSchema};

use crate::{call::validate_schema_pin, errors};

pub const MAX_PINS: usize = 64;

/// `tool.views` params. Pins are opaque and need not be a particular pin encoding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ViewsRequest {
    pub pins: Vec<String>,
}

impl ViewsRequest {
    pub fn new(pins: Vec<String>) -> Self {
        Self { pins }
    }

    pub fn validate(&self) -> Result<(), ErrorBody> {
        if self.pins.len() > MAX_PINS {
            return Err(errors::invalid_request("pins", "at most 64 pins"));
        }
        for (index, pin) in self.pins.iter().enumerate() {
            validate_schema_pin(pin).map_err(|_| {
                errors::invalid_request(&format!("pins[{index}]"), "invalid opaque pin")
            })?;
        }
        Ok(())
    }
}

/// A page read admitted by this view; remaining members belong to the provider.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ViewRead {
    pub op: String,
    #[serde(flatten)]
    pub extensions: Map<String, Value>,
}

impl ViewRead {
    pub fn new(op: impl Into<String>) -> Self {
        Self {
            op: op.into(),
            extensions: Map::new(),
        }
    }
}

/// Provider schema, the detail reads the app server may call, and a template
/// that only the app server interprets.
///
/// For one pin, `record_schema` and `reads` never change. `template` may be
/// replaced, but only together with a higher `template_revision` (§2.1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ViewManifest {
    pub record_schema: RecordSchema,
    pub reads: Vec<ViewRead>,
    pub template: Value,
    /// Raised by the provider each time it replaces this pin's template.
    /// Defaults to 0 and is omitted when 0, so a view without revisions
    /// serializes with the same members as before this field existed.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub template_revision: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// How a refetched view relates to the view an app server already keeps for
/// the same pin (`ViewManifest::compare_with_kept`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ViewChange {
    /// Identical to the kept view.
    Same,
    /// Same record schema and reads with a higher template revision: keep
    /// the fetched view in place of the kept one, its revision included.
    NewerTemplate,
    /// Same record schema and reads with a lower template revision: keep the
    /// kept view.
    Stale,
    /// A changed record schema or reads, whatever the revision, or the kept
    /// revision with a different template. That breaks the provider's §2.1
    /// obligation; keep the kept view.
    Conflict,
}

impl ViewManifest {
    pub fn new(record_schema: RecordSchema, reads: Vec<ViewRead>, template: Value) -> Self {
        Self {
            record_schema,
            reads,
            template,
            template_revision: 0,
        }
    }

    pub fn with_template_revision(mut self, template_revision: u64) -> Self {
        self.template_revision = template_revision;
        self
    }

    /// Compare this freshly fetched view with the one kept for the same pin.
    pub fn compare_with_kept(&self, kept: &ViewManifest) -> ViewChange {
        if self.record_schema != kept.record_schema || self.reads != kept.reads {
            return ViewChange::Conflict;
        }
        match self.template_revision.cmp(&kept.template_revision) {
            std::cmp::Ordering::Greater => ViewChange::NewerTemplate,
            std::cmp::Ordering::Less => ViewChange::Stale,
            std::cmp::Ordering::Equal if self.template == kept.template => ViewChange::Same,
            std::cmp::Ordering::Equal => ViewChange::Conflict,
        }
    }
}

/// A view or the exact string `unknown`; no interpretation of template JSON.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ViewEntry {
    Manifest(ViewManifest),
    Unknown,
}

impl Serialize for ViewEntry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Manifest(view) => view.serialize(serializer),
            Self::Unknown => serializer.serialize_str("unknown"),
        }
    }
}

impl<'de> Deserialize<'de> for ViewEntry {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        if value.as_str() == Some("unknown") {
            return Ok(Self::Unknown);
        }
        serde_json::from_value(value)
            .map(Self::Manifest)
            .map_err(serde::de::Error::custom)
    }
}

/// Bare `tool.views` answer, keyed by the requested pins.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ViewsReply {
    pub views: BTreeMap<String, ViewEntry>,
}

impl ViewsReply {
    pub fn new(views: BTreeMap<String, ViewEntry>) -> Self {
        Self { views }
    }

    /// Require exactly one entry for each requested pin, and valid schemas.
    pub fn validate_for(&self, request: &ViewsRequest) -> Result<(), ErrorBody> {
        request.validate()?;
        if request.pins.iter().any(|pin| !self.views.contains_key(pin))
            || self.views.keys().any(|pin| !request.pins.contains(pin))
        {
            return Err(errors::invalid_request(
                "views",
                "reply pins differ from request",
            ));
        }
        for (pin, entry) in &self.views {
            if let ViewEntry::Manifest(view) = entry {
                view.record_schema.validate().map_err(|problem| {
                    errors::invalid_request(
                        &format!("views.{pin}.{}", problem.field),
                        "invalid record schema",
                    )
                })?;
            }
        }
        Ok(())
    }
}
