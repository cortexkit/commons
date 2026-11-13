use std::collections::BTreeMap;

use serde_json::Value;

use crate::PersonaError;

/// Names and container kinds declared for guidance facts.
///
/// This is a name-availability declaration, not a JSON Schema validator. Scalar
/// types are interchangeable for path checking. Objects have named members;
/// lists contain only scalars. Optional properties still declare available names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactsShape(pub(super) Shape);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Shape {
    Object(BTreeMap<String, Shape>),
    List,
    Scalar,
}

impl FactsShape {
    /// Parse `type`, `properties` and `items` from a small JSON Schema subset.
    ///
    /// Supported types are object, array, string, boolean, integer and number.
    /// `type` is required at every node; object `properties` defaults to empty;
    /// arrays require scalar `items`. Boolean schemas and type unions are refused.
    /// `required`, `additionalProperties`, `minLength`, `uniqueItems`, `$schema`,
    /// `$id`, `title`, `description` and all `x-*` annotations are accepted and
    /// ignored, without interpreting their values. Every other keyword is
    /// refused by name, at any modelled node. A schema passed to
    /// [`super::GuidanceSet::check`] must describe an object at its root.
    pub fn from_json_schema(schema: &Value) -> Result<Self, PersonaError> {
        parse(schema, "$").map(Self)
    }
}

fn parse(schema: &Value, path: &str) -> Result<Shape, PersonaError> {
    let fail = |message: String| PersonaError::FactsSchema {
        path: path.into(),
        message,
    };
    let node = schema
        .as_object()
        .ok_or_else(|| fail("expected a schema object".into()))?;
    for key in node.keys() {
        if !matches!(
            key.as_str(),
            "type"
                | "properties"
                | "items"
                | "required"
                | "additionalProperties"
                | "minLength"
                | "uniqueItems"
                | "$schema"
                | "$id"
                | "title"
                | "description"
        ) && !key.starts_with("x-")
        {
            return Err(fail(format!("unsupported keyword `{key}`")));
        }
    }
    let kind = node
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| fail("`type` must be one supported type name".into()))?;
    if kind != "object" && node.contains_key("properties") {
        return Err(fail("`properties` requires type object".into()));
    }
    if kind != "array" && node.contains_key("items") {
        return Err(fail("`items` requires type array".into()));
    }
    match kind {
        "object" => {
            let mut members = BTreeMap::new();
            if let Some(properties) = node.get("properties") {
                let properties = properties
                    .as_object()
                    .ok_or_else(|| fail("`properties` must be an object".into()))?;
                for (name, child) in properties {
                    members.insert(
                        name.clone(),
                        parse(child, &format!("{path}.properties[{name:?}]"))?,
                    );
                }
            }
            Ok(Shape::Object(members))
        }
        "array" => {
            let items = node
                .get("items")
                .ok_or_else(|| fail("array requires `items`".into()))?;
            if parse(items, &format!("{path}.items"))? != Shape::Scalar {
                return Err(fail(
                    "`items` must describe a scalar, not an object or array".into(),
                ));
            }
            Ok(Shape::List)
        }
        "string" | "boolean" | "integer" | "number" => Ok(Shape::Scalar),
        other => Err(fail(format!("unsupported `type` `{other}`"))),
    }
}
