//! Atomic validation of the independent `media-provider/v1` catalog slot.

use std::collections::BTreeSet;

use cortexkit_role_media::describe::{Engine, Kind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A media declaration is returned only after every engine has passed.
/// The caller must separately require the catalog's nonce verification.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EngineDeclarations {
    pub engines: Vec<Engine>,
}

impl EngineDeclarations {
    pub fn new(engines: Vec<Engine>) -> Self {
        Self { engines }
    }
}

/// A health diagnostic, not a partially applicable declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EngineDeclarationError {
    pub code: String,
    pub field: String,
}

impl EngineDeclarationError {
    pub fn new(code: impl Into<String>, field: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            field: field.into(),
        }
    }
}

impl std::fmt::Display for EngineDeclarationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.code, self.field)
    }
}

impl std::error::Error for EngineDeclarationError {}

/// Read engines from the media contract's `media-provider/v1` catalog slot.
///
/// The slot belongs to the media contract. Routing reads only `engines`, which
/// is required. The media provider may carry other members, including
/// describe's group facts; those members are ignored. One malformed engine
/// still rejects the whole slot.
pub fn parse_engine_declarations(
    value: &Value,
) -> Result<EngineDeclarations, EngineDeclarationError> {
    let object = value
        .as_object()
        .ok_or_else(|| EngineDeclarationError::new("invalid_params", "$"))?;
    let engines = object
        .get("engines")
        .ok_or_else(|| EngineDeclarationError::new("invalid_params", "engines"))?;
    Ok(EngineDeclarations::new(parse_engines(
        engines, "engines", false,
    )?))
}

pub(crate) fn parse_engines(
    value: &Value,
    path: &str,
    caller_local: bool,
) -> Result<Vec<Engine>, EngineDeclarationError> {
    let values = value
        .as_array()
        .ok_or_else(|| EngineDeclarationError::new("invalid_params", path))?;
    let mut engines = Vec::with_capacity(values.len());
    let mut keys = BTreeSet::new();
    for (index, value) in values.iter().enumerate() {
        let path = format!("{path}[{index}]");
        let error =
            |code: &str, field: &str| EngineDeclarationError::new(code, format!("{path}.{field}"));
        let object = value
            .as_object()
            .ok_or_else(|| EngineDeclarationError::new("invalid_params", &path))?;
        const STRINGS: &[&str] = &["kind", "provider", "model", "latency_class"];
        const LISTS: &[&str] = &[
            "formats",
            "reference_formats",
            "languages",
            "voices",
            "sizes",
            "resolutions",
            "qualities",
        ];
        if let Some(key) = object.keys().find(|key| {
            !STRINGS.contains(&key.as_str())
                && !LISTS.contains(&key.as_str())
                && !["local", "max_input_bytes", "max_duration_ms", "bias_limit"]
                    .contains(&key.as_str())
        }) {
            return Err(error("unknown_member", key));
        }
        for field in STRINGS {
            if !object.get(*field).is_some_and(Value::is_string) {
                return Err(error("invalid_params", field));
            }
        }
        for field in ["provider", "model"] {
            if object[field]
                .as_str()
                .is_some_and(|v| v.chars().count() > 256)
            {
                return Err(error("fact_too_long", field));
            }
        }
        if !object.get("local").is_some_and(Value::is_boolean)
            || (caller_local && object["local"] != true)
        {
            return Err(error("invalid_params", "local"));
        }
        for field in LISTS {
            match object.get(*field) {
                None if *field != "formats" => continue,
                Some(Value::Null) if *field != "formats" => continue,
                Some(Value::Array(list)) => {
                    for (i, entry) in list.iter().enumerate() {
                        let field = format!("{field}[{i}]");
                        let text = entry
                            .as_str()
                            .ok_or_else(|| error("invalid_params", &field))?;
                        if text.chars().count() > 256 {
                            return Err(error("fact_too_long", &field));
                        }
                    }
                    if *field == "formats" && list.is_empty() {
                        return Err(error("formats_empty", field));
                    }
                }
                _ => return Err(error("invalid_params", field)),
            }
        }
        for field in ["max_input_bytes", "max_duration_ms"] {
            if object
                .get(field)
                .is_some_and(|value| !value.is_null() && value.as_u64().is_none())
            {
                return Err(error("invalid_params", field));
            }
        }
        if let Some(limit) = object.get("bias_limit") {
            let limit = limit
                .as_object()
                .ok_or_else(|| error("bias_limit_invalid", "bias_limit"))?;
            if let Some(key) = limit
                .keys()
                .find(|key| !["unit", "max"].contains(&key.as_str()))
            {
                return Err(error("unknown_member", &format!("bias_limit.{key}")));
            }
            if !limit.get("unit").is_some_and(Value::is_string) {
                return Err(error("invalid_params", "bias_limit.unit"));
            }
            if !limit
                .get("max")
                .and_then(Value::as_u64)
                .is_some_and(|max| max >= 1)
            {
                return Err(error("bias_limit_invalid", "bias_limit.max"));
            }
        }
        let engine: Engine = serde_json::from_value(value.clone())
            .map_err(|_| EngineDeclarationError::new("invalid_params", &path))?;
        if engine.local && engine.kind == Kind::Realtime {
            return Err(error("local_realtime_engine", "local"));
        }
        if !keys.insert((
            engine.kind.clone(),
            engine.provider.clone(),
            engine.model.clone(),
        )) {
            return Err(error("duplicate_engine", "model"));
        }
        engines.push(engine);
    }
    Ok(engines)
}
