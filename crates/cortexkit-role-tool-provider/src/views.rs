//! Optional display views, fetched by the display server off the write path.
//! Under a tool pin, reads stay fixed and the record schema can only grow.
//! Updates require a higher `template_revision` (CONTRACT §2.1).

mod notice;

use std::collections::BTreeMap;

use serde::{ser::SerializeStruct, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use subc_protocol::ErrorBody;

pub use cortexkit_role_tool_display::{FieldKind, RecordSchema};
pub use notice::{NoticeView, NoticeViewDefinition, NoticeViewTemplates};

use crate::{
    call::{
        validate_call_key, validate_schema_pin, SchemaPin, ToolMajorPin,
        VERSIONED_SCHEMA_PIN_PREFIX,
    },
    catalog::{decode_version, schema_digest, CatalogTool, ToolVersion},
    errors,
};

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

/// The catalog definition needed to recover a pin's input schema after the
/// tool has left the provider's current catalog.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ViewTool {
    pub name: String,
    pub input_schema: Value,
    pub semantics: u64,
    pub version: Option<ToolVersion>,
}

impl ViewTool {
    pub fn new(name: impl Into<String>, input_schema: Value, semantics: u64) -> Self {
        Self {
            name: name.into(),
            input_schema,
            semantics,
            version: None,
        }
    }

    pub fn new_versioned(
        name: impl Into<String>,
        input_schema: Value,
        version: ToolVersion,
    ) -> Self {
        Self {
            name: name.into(),
            input_schema,
            semantics: 0,
            version: Some(version),
        }
    }

    /// Compute the canonical pin using the catalog's structural schema
    /// digest, not a digest supplied by the view's writer.
    pub fn schema_pin(&self) -> Result<String, ErrorBody> {
        let digest = schema_digest(&self.input_schema)
            .map_err(|error| errors::invalid_request("tool.input_schema", error.to_string()))?;
        match self.version {
            Some(version) => ToolMajorPin::new(&self.name, digest, version.major).encode(),
            None => SchemaPin::new(&self.name, digest, self.semantics).encode(),
        }
        .map_err(|error| errors::invalid_request("tool", error.to_string()))
    }
}

impl From<&CatalogTool> for ViewTool {
    fn from(tool: &CatalogTool) -> Self {
        match tool.version {
            Some(version) => Self::new_versioned(&tool.name, tool.input_schema.clone(), version),
            None => Self::new(&tool.name, tool.input_schema.clone(), tool.semantics),
        }
    }
}

impl Serialize for ViewTool {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut wire = serializer.serialize_struct("ViewTool", 3)?;
        wire.serialize_field("name", &self.name)?;
        wire.serialize_field("input_schema", &self.input_schema)?;
        if let Some(version) = self.version {
            wire.serialize_field("version", &version)?;
        } else {
            wire.serialize_field("semantics", &self.semantics)?;
        }
        wire.end()
    }
}

impl<'de> Deserialize<'de> for ViewTool {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            name: String,
            input_schema: Value,
            semantics: Option<u64>,
            #[serde(default, deserialize_with = "decode_version")]
            version: Option<ToolVersion>,
        }
        let wire = Wire::deserialize(decoder)?;
        Ok(match wire.version {
            Some(version) => Self::new_versioned(wire.name, wire.input_schema, version),
            None => Self::new(
                wire.name,
                wire.input_schema,
                wire.semantics
                    .ok_or_else(|| serde::de::Error::missing_field("semantics"))?,
            ),
        })
    }
}

/// The wake record paired with a tool row for the same event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ViewHalfOf {
    /// Wake kind as defined by tool-display. Readers retain unfamiliar kinds.
    pub kind: String,
    /// Writers require an opaque family for `notice` and omit it otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<String>,
}

impl ViewHalfOf {
    pub fn new(kind: impl Into<String>, family: Option<String>) -> Self {
        Self {
            kind: kind.into(),
            family,
        }
    }

    fn check(&self) -> Result<(), (&'static str, &'static str)> {
        match &self.family {
            Some(family) if validate_call_key(family).is_err() => {
                Err(("family", "invalid opaque family"))
            }
            None if self.kind == "notice" => Err(("family", "required for notice")),
            _ => Ok(()),
        }
    }

    fn check_writer(&self) -> Result<(), (&'static str, &'static str)> {
        self.check()?;
        match self.kind.as_str() {
            "notice" => Ok(()),
            "peer_message" | "room_post" | "worker_question" | "work_result"
            | "sidekick_answer" | "approval_card" | "ask_answer" => match &self.family {
                Some(_) => Err(("family", "only allowed for notice")),
                None => Ok(()),
            },
            _ => Err(("kind", "unknown wake kind")),
        }
    }
}

/// Provider schema, the detail reads the app server may call, and a template
/// that only the app server interprets.
///
/// For one pin, `reads` never change. A higher `template_revision` can replace
/// the template, refresh description text, or extend `record_schema` (§2.1).
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
    /// The tool definition behind this pin. Absent on older providers' `tp1`
    /// views. The pin fixes the name, structure and semantics or major.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ViewTool>,
    /// The paired wake record. An absent marker means an ordinary row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half_of: Option<Box<ViewHalfOf>>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// How a refetched view relates to the view an app server already keeps for
/// the same pin (`ViewManifest::compare_with_kept`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ViewChange {
    /// Same schema, reads, template revision and template as the kept view,
    /// with no change to a previously pinned tool definition.
    Same,
    /// Higher template revision with unchanged reads and an unchanged or
    /// grown record schema: replace the kept view.
    NewerTemplate,
    /// Lower template revision with an unchanged or older record schema.
    Stale,
    /// An incompatible change or a changed definition at the kept revision.
    Conflict,
}

impl ViewManifest {
    pub fn new(record_schema: RecordSchema, reads: Vec<ViewRead>, template: Value) -> Self {
        Self {
            record_schema,
            reads,
            template,
            template_revision: 0,
            tool: None,
            half_of: None,
        }
    }

    pub fn with_template_revision(mut self, template_revision: u64) -> Self {
        self.template_revision = template_revision;
        self
    }

    pub fn with_tool(mut self, tool: ViewTool) -> Self {
        self.tool = Some(tool);
        self
    }

    pub fn with_half_of(mut self, half_of: ViewHalfOf) -> Self {
        self.half_of = Some(Box::new(half_of));
        self
    }

    /// Return `true` when `tool` recomputes to `pin`, or `false` when `tool`
    /// is absent and the display server must use the current catalog check.
    /// It errors when the schema can't be canonicalized, the pin can't be
    /// encoded, or the recomputed pin differs: treat the view as `unknown`,
    /// never fall back to catalog membership (tool-display §6). It checks the
    /// pin binding only, not that `input_schema` is a valid argument schema,
    /// and not the writer or the runner's stored call pin.
    pub fn check_pin(&self, pin: &str) -> Result<bool, ErrorBody> {
        let Some(tool) = &self.tool else {
            if pin.starts_with(VERSIONED_SCHEMA_PIN_PREFIX) {
                return Err(errors::invalid_request(
                    "tool",
                    "a tp2 view requires a versioned tool",
                ));
            }
            return Ok(false);
        };
        if tool.schema_pin()? != pin {
            return Err(errors::invalid_request(
                "tool",
                "tool does not match the requested pin",
            ));
        }
        Ok(true)
    }

    /// Compare this freshly fetched view with the one kept for the same pin.
    pub fn compare_with_kept(&self, kept: &ViewManifest) -> ViewChange {
        let same_schema = self.record_schema == kept.record_schema;
        if self.reads != kept.reads {
            return ViewChange::Conflict;
        }
        let same_tool = match (&self.tool, &kept.tool) {
            (Some(fetched), Some(held)) => {
                let (Ok(fetched_pin), Ok(held_pin)) = (fetched.schema_pin(), held.schema_pin())
                else {
                    return ViewChange::Conflict;
                };
                if fetched_pin != held_pin {
                    return ViewChange::Conflict;
                }
                if self.template_revision > kept.template_revision
                    && fetched.version != held.version
                    && (fetched.version < held.version
                        || (same_schema && fetched.input_schema == held.input_schema))
                {
                    return ViewChange::Conflict;
                }
                fetched == held
            }
            (None, Some(_)) => return ViewChange::Conflict,
            // Older views can gain a checked tool definition without changing revision.
            _ => true,
        };
        match self.template_revision.cmp(&kept.template_revision) {
            std::cmp::Ordering::Greater
                if same_schema || self.record_schema.is_growth_of(&kept.record_schema) =>
            {
                ViewChange::NewerTemplate
            }
            std::cmp::Ordering::Less
                if same_schema || kept.record_schema.is_growth_of(&self.record_schema) =>
            {
                ViewChange::Stale
            }
            std::cmp::Ordering::Equal
                if same_schema && same_tool && self.template == kept.template =>
            {
                ViewChange::Same
            }
            _ => ViewChange::Conflict,
        }
    }
}

/// A tool view, notice view, or the exact string `unknown`.
// Almost every entry is a manifest and `Unknown` carries nothing, so boxing
// the manifest would only add an allocation per entry and change the public
// pattern `ViewEntry::Manifest(manifest)`.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ViewEntry {
    Manifest(ViewManifest),
    Notice(NoticeView),
    Unknown,
}

impl Serialize for ViewEntry {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Manifest(view) => view.serialize(serializer),
            Self::Notice(view) => view.serialize(serializer),
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
        // Tool manifests have always ignored additional members. A `notice`
        // member cannot change how a manifest with its required members decodes.
        if value.get("notice").is_some()
            && !(value.get("record_schema").is_some() && value.get("reads").is_some())
        {
            return serde_json::from_value(value)
                .map(Self::Notice)
                .map_err(serde::de::Error::custom);
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

    /// Check pin coverage, schemas and paired wake markers, accepting unknown
    /// wake kinds. Check notice entries against their pins; use
    /// [`NoticeView::check_pin`] to also verify the publisher's attested id.
    pub fn validate_for(&self, request: &ViewsRequest) -> Result<(), ErrorBody> {
        self.validate_with(request, ViewHalfOf::check)
    }

    /// Check a provider's reply before sending it. Writers use only known wake
    /// kinds and must omit the family for every kind other than `notice`.
    pub fn validate_writer_for(&self, request: &ViewsRequest) -> Result<(), ErrorBody> {
        self.validate_with(request, ViewHalfOf::check_writer)
    }

    fn validate_with(
        &self,
        request: &ViewsRequest,
        check_half_of: fn(&ViewHalfOf) -> Result<(), (&'static str, &'static str)>,
    ) -> Result<(), ErrorBody> {
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
            if let ViewEntry::Notice(view) = entry {
                view.check_entry(pin)?;
            }
            if let ViewEntry::Manifest(view) = entry {
                if cortexkit_role_tool_display::parse_notice_view_key(pin).is_some() {
                    return Err(errors::invalid_request(
                        &format!("views.{pin}"),
                        "a notice pin requires a notice view",
                    ));
                }
                if pin.starts_with(VERSIONED_SCHEMA_PIN_PREFIX) {
                    view.check_pin(pin)?;
                }
                view.record_schema.validate().map_err(|problem| {
                    errors::invalid_request(
                        &format!("views.{pin}.{}", problem.field),
                        "invalid record schema",
                    )
                })?;
                if let Some(half_of) = &view.half_of {
                    check_half_of(half_of).map_err(|(field, reason)| {
                        errors::invalid_request(&format!("views.{pin}.half_of.{field}"), reason)
                    })?;
                }
            }
        }
        Ok(())
    }
}
