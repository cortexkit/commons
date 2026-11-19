//! Record and completion fields, checked against their declared kinds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use subc_protocol::tool_call::validate_call_key;

use crate::InvalidParams;

/// A provider-declared extension field's type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FieldKind {
    Code,
    Id,
    AgentId,
    Timestamp,
    Integer,
    Boolean,
    ProjectPath,
    WebOrigin,
    AppId,
    /// A bounded array of line objects, allowed only in completion fields.
    TextLines,
}

/// Schema for provider extensions; standard fields always have fixed validators.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RecordSchema {
    pub name: String,
    pub revision: u64,
    /// Extension fields shared by records and completions.
    pub fields: BTreeMap<String, FieldKind>,
    /// Additional extension fields accepted only in completions.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub completion_fields: BTreeMap<String, FieldKind>,
}

impl RecordSchema {
    pub fn new(
        name: impl Into<String>,
        revision: u64,
        fields: BTreeMap<String, FieldKind>,
    ) -> Self {
        Self {
            name: name.into(),
            revision,
            fields,
            completion_fields: BTreeMap::new(),
        }
    }

    /// Check that extensions are disjoint, do not redefine standard fields,
    /// and declare `text_lines` only in completion fields.
    pub fn validate(&self) -> Result<(), InvalidParams> {
        for (field, kind) in &self.fields {
            if is_standard(field) || *kind == FieldKind::TextLines {
                return Err(InvalidParams::new(format!("record_schema.fields.{field}")));
            }
        }
        for field in self.completion_fields.keys() {
            if is_standard(field) || self.fields.contains_key(field) {
                return Err(InvalidParams::new(format!(
                    "record_schema.completion_fields.{field}"
                )));
            }
        }
        Ok(())
    }
}

/// `^[a-z][a-z0-9_]{0,63}$`, for machine-readable reasons and codes.
pub fn validate_code(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// The protocol's opaque-field rule, without interpreting the identifier.
pub fn validate_id(value: &str) -> bool {
    validate_call_key(value).is_ok()
}

/// Agent identifiers obey exactly the opaque-field rule.
pub fn validate_agent_id(value: &str) -> bool {
    validate_id(value)
}

/// Integer milliseconds since the Unix epoch, including times before it.
pub fn validate_timestamp(value: &Value) -> bool {
    validate_integer(value)
}

/// An i64 JSON number, not a floating-point number or a string.
pub fn validate_integer(value: &Value) -> bool {
    value.as_i64().is_some()
}

pub fn validate_boolean(value: &Value) -> bool {
    value.is_boolean()
}

/// A bounded project-relative path, never a host filesystem path.
pub fn validate_project_path(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && !value.starts_with('/')
        && !value.contains(['\\', '\0'])
        && !(value.as_bytes().get(1) == Some(&b':') && value.as_bytes()[0].is_ascii_alphabetic())
        && !value.split('/').any(|segment| segment == "..")
}

/// A lowercase `scheme://host[:port]`, without credentials or URL suffixes.
pub fn validate_web_origin(value: &str) -> bool {
    if value != value.to_lowercase() {
        return false;
    }
    let Some((scheme, authority)) = value.split_once("://") else {
        return false;
    };
    if scheme.is_empty()
        || authority.is_empty()
        || authority.ends_with(':')
        || authority.contains(['/', '?', '#', '@', '\\'])
        || value.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return false;
    }
    url::Url::parse(value).is_ok_and(|url| {
        url.host_str().is_some() && url.username().is_empty() && url.password().is_none()
    })
}

/// A reverse-DNS id or executable base name, never a path.
pub fn validate_app_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

impl FieldKind {
    /// Validate one scalar without JSON coercions. `TextLines` has no scalar form.
    pub fn validate_scalar(self, value: &Value) -> bool {
        match self {
            Self::Timestamp => validate_timestamp(value),
            Self::Integer => validate_integer(value),
            Self::Boolean => validate_boolean(value),
            _ => value.as_str().is_some_and(|value| match self {
                Self::Code => validate_code(value),
                Self::Id => validate_id(value),
                Self::AgentId => validate_agent_id(value),
                Self::ProjectPath => validate_project_path(value),
                Self::WebOrigin => validate_web_origin(value),
                Self::AppId => validate_app_id(value),
                _ => false,
            }),
        }
    }

    /// Validate a field value: one scalar or a nonempty array of at most 64
    /// scalars for scalar kinds, or a bounded array of line objects for `TextLines`.
    pub fn validate(self, value: &Value) -> bool {
        if self == Self::TextLines {
            return validate_text_lines("text_lines", value).is_ok();
        }
        match value.as_array() {
            Some(items) => {
                (1..=64).contains(&items.len())
                    && items.iter().all(|item| self.validate_scalar(item))
            }
            None => self.validate_scalar(value),
        }
    }
}

fn validate_text_lines(field: &str, value: &Value) -> Result<(), InvalidParams> {
    let items = value.as_array().ok_or_else(|| InvalidParams::new(field))?;
    if items.len() > 3 {
        return Err(InvalidParams::new(format!("{field}[3]")));
    }
    for (index, item) in items.iter().enumerate() {
        let item = item
            .as_object()
            .filter(|item| {
                item.keys()
                    .all(|key| matches!(key.as_str(), "text" | "tone"))
            })
            .ok_or_else(|| InvalidParams::new(format!("{field}[{index}]")))?;
        let valid_text = item
            .get("text")
            .and_then(Value::as_str)
            .is_some_and(|text| {
                !text.is_empty()
                    && text
                        .chars()
                        .enumerate()
                        .all(|(index, c)| index < 200 && !c.is_control())
            });
        if !valid_text {
            return Err(InvalidParams::new(format!("{field}[{index}].text")));
        }
        if item
            .get("tone")
            .is_some_and(|tone| !matches!(tone.as_str(), Some("muted" | "danger")))
        {
            return Err(InvalidParams::new(format!("{field}[{index}].tone")));
        }
    }
    Ok(())
}

fn is_standard(field: &str) -> bool {
    matches!(
        field,
        "durationMs" | "outcome" | "reason" | "queuedMs" | "heldMs" | "location"
    )
}

/// Check required record fields and every declared extension, including arrays.
pub fn validate_record(
    body: &Map<String, Value>,
    schema: &RecordSchema,
) -> Result<(), InvalidParams> {
    validate_flat(body, schema, true)
}

/// A completion requires an outcome but may omit durationMs.
pub fn validate_completion(
    body: &Map<String, Value>,
    schema: &RecordSchema,
) -> Result<(), InvalidParams> {
    validate_flat(body, schema, false)
}

fn validate_flat(
    body: &Map<String, Value>,
    schema: &RecordSchema,
    require_duration: bool,
) -> Result<(), InvalidParams> {
    schema.validate()?;
    let required: &[&str] = if require_duration {
        &["durationMs", "outcome"]
    } else {
        &["outcome"]
    };
    for &field in required {
        if !body.contains_key(field) {
            return Err(InvalidParams::new(field));
        }
    }
    for (field, value) in body {
        let valid = match field.as_str() {
            "durationMs" | "queuedMs" | "heldMs" => value.as_i64().is_some_and(|ms| ms >= 0),
            "outcome" => value
                .as_str()
                .is_some_and(|s| matches!(s, "success" | "error" | "partial" | "held")),
            "reason" => value.as_str().is_some_and(validate_code),
            "location" => value.as_str() == Some("outside_project"),
            _ => {
                let kind = schema.fields.get(field).or_else(|| {
                    if require_duration {
                        None
                    } else {
                        schema.completion_fields.get(field)
                    }
                });
                match kind {
                    Some(FieldKind::TextLines) => {
                        validate_text_lines(field, value)?;
                        true
                    }
                    Some(kind) => kind.validate(value),
                    None => false,
                }
            }
        };
        if !valid {
            return Err(InvalidParams::new(field));
        }
    }
    Ok(())
}

/// Details are free-form objects; standard cut metadata has fixed types.
pub fn validate_detail(body: &Map<String, Value>) -> Result<(), InvalidParams> {
    for (field, value) in body {
        let valid = match field.as_str() {
            "truncated" => value.as_bool() == Some(true),
            "omittedItems" | "omittedBytes" => value.as_u64().is_some(),
            _ => true,
        };
        if !valid {
            return Err(InvalidParams::new(field));
        }
    }
    Ok(())
}
