//! Portability lint for tool input schemas.
//!
//! JSON Schema lets a property schema leave out `type`: `{"enum": ["a", "b"]}`
//! is valid, and some model providers accept it. Others refuse the whole
//! request when any property lacks a `type`, so a tool whose schema relies on
//! the omission works with one model and breaks every request routed to
//! another. [`lint_input_schema`] finds those properties so a provider can fix
//! its schema before shipping it.
//!
//! The lint only reports. Decoding never calls it, no check in this crate
//! refuses a schema because of it, and nothing here rewrites a schema.

use serde_json::Value;

/// What a [`SchemaLint`] finding says is missing.
///
/// `#[non_exhaustive]` so a later portability rule can add a variant without
/// breaking callers that match on this.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemaLintKind {
    /// The schema declares no `type`, and is neither a combinator (`anyOf`,
    /// `oneOf`, `allOf`) nor a `$ref`, whose type comes from elsewhere.
    MissingType,
}

/// One portability finding in a tool's `input_schema`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SchemaLint {
    /// RFC 6901 JSON pointer from the schema root to the offending schema,
    /// such as `/properties/surface` or `/properties/tags/items`.
    pub path: String,
    pub kind: SchemaLintKind,
}

impl std::fmt::Display for SchemaLint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            SchemaLintKind::MissingType => write!(
                f,
                "{}: declares no type; some model providers refuse a property without one",
                self.path
            ),
        }
    }
}

/// Keywords that make a schema's type come from its branches.
const COMBINATOR_KEYWORDS: &[&str] = &["anyOf", "oneOf", "allOf"];

/// Report every property schema in `schema` that declares no `type`.
///
/// Starting at the root, the walk visits each schema under `properties`, the
/// `items` schema (or each entry of a draft-4 `items` array), each
/// `prefixItems` entry, and `additionalProperties` when it is an object, and
/// repeats the walk inside each of them. The root itself is not checked: its
/// shape is `check_flat_schema`'s concern. A visited schema is reported when
/// it has no `type` and no `anyOf`, `oneOf`, `allOf` or `$ref`. A boolean
/// schema such as `"name": true` cannot declare a type and is reported too;
/// a boolean `additionalProperties` is the usual open-or-closed flag and is
/// not.
///
/// Branches of `anyOf`, `oneOf` and `allOf`, and `$defs` reached only through
/// `$ref`, are not walked: a branch often adds constraints without
/// repeating the type, so reporting it would flag schemas every model accepts.
///
/// The result is empty for a portable schema, and for a value that is not a
/// schema object at all. The function never fails and never changes anything.
pub fn lint_input_schema(schema: &Value) -> Vec<SchemaLint> {
    let mut findings = Vec::new();
    walk_children(schema, "", &mut findings);
    findings
}

/// Check one schema reached through a structural keyword, then walk inside it.
fn visit(schema: &Value, path: String, findings: &mut Vec<SchemaLint>) {
    match schema {
        Value::Object(object) => {
            let typed = object
                .get("type")
                .is_some_and(|ty| ty.is_string() || ty.as_array().is_some_and(|a| !a.is_empty()));
            let delegated = object.contains_key("$ref")
                || COMBINATOR_KEYWORDS
                    .iter()
                    .any(|keyword| object.contains_key(*keyword));
            if !typed && !delegated {
                findings.push(SchemaLint {
                    path: path.clone(),
                    kind: SchemaLintKind::MissingType,
                });
            }
            walk_children(schema, &path, findings);
        }
        Value::Bool(_) => findings.push(SchemaLint {
            path,
            kind: SchemaLintKind::MissingType,
        }),
        // Not a schema; other checks own malformed shapes.
        _ => {}
    }
}

/// Visit the subschemas `schema` holds under the keywords the lint walks.
fn walk_children(schema: &Value, path: &str, findings: &mut Vec<SchemaLint>) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(Value::Object(properties)) = object.get("properties") {
        for (name, property) in properties {
            visit(
                property,
                format!("{path}/properties/{}", escape_pointer_token(name)),
                findings,
            );
        }
    }
    match object.get("items") {
        Some(Value::Array(entries)) => {
            for (index, entry) in entries.iter().enumerate() {
                visit(entry, format!("{path}/items/{index}"), findings);
            }
        }
        Some(items) => visit(items, format!("{path}/items"), findings),
        None => {}
    }
    if let Some(Value::Array(entries)) = object.get("prefixItems") {
        for (index, entry) in entries.iter().enumerate() {
            visit(entry, format!("{path}/prefixItems/{index}"), findings);
        }
    }
    if let Some(additional @ Value::Object(_)) = object.get("additionalProperties") {
        visit(additional, format!("{path}/additionalProperties"), findings);
    }
}

/// Escape one reference token for a JSON pointer (RFC 6901 section 3).
fn escape_pointer_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The reported paths, sorted so the assertion does not depend on whether
    /// the build keeps JSON object members in insertion order.
    fn paths(schema: Value) -> Vec<String> {
        let mut paths: Vec<String> = lint_input_schema(&schema)
            .into_iter()
            .map(|lint| {
                assert_eq!(lint.kind, SchemaLintKind::MissingType);
                lint.path
            })
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn an_enum_without_a_type_is_reported_by_path() {
        let lints = lint_input_schema(&json!({
            "type": "object",
            "properties": { "surface": { "enum": ["a", "b"] } }
        }));
        assert_eq!(
            lints,
            vec![SchemaLint {
                path: "/properties/surface".into(),
                kind: SchemaLintKind::MissingType,
            }]
        );
        assert!(lints[0].to_string().contains("/properties/surface"));
        assert!(lints[0].to_string().contains("declares no type"));
    }

    #[test]
    fn typed_properties_combinators_and_refs_are_not_reported() {
        assert!(paths(json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" },
                "either": { "type": ["string", "null"] },
                "any": { "anyOf": [{ "type": "string" }, { "enum": [1] }] },
                "one": { "oneOf": [{ "type": "integer" }] },
                "all": { "allOf": [{ "minimum": 1 }] },
                "linked": { "$ref": "#/$defs/thing" }
            },
            "$defs": { "thing": { "enum": ["unchecked"] } }
        }))
        .is_empty());
    }

    #[test]
    fn nested_properties_items_prefix_items_and_additional_properties_are_walked() {
        assert_eq!(
            paths(json!({
                "type": "object",
                "properties": {
                    "outer": {
                        "type": "object",
                        "properties": { "inner": { "const": 1 } }
                    },
                    "list": { "type": "array", "items": { "enum": ["x"] } },
                    "tuple": {
                        "type": "array",
                        "prefixItems": [{ "type": "string" }, { "description": "untyped" }]
                    },
                    "old_tuple": { "type": "array", "items": [{ "type": "string" }, {}] },
                    "map": {
                        "type": "object",
                        "additionalProperties": { "enum": [true] }
                    },
                    "closed": { "type": "object", "additionalProperties": false },
                    "deep": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "leaf": { "enum": [1] } }
                        }
                    }
                }
            })),
            [
                "/properties/deep/items/properties/leaf",
                "/properties/list/items",
                "/properties/map/additionalProperties",
                "/properties/old_tuple/items/1",
                "/properties/outer/properties/inner",
                "/properties/tuple/prefixItems/1",
            ]
        );
    }

    #[test]
    fn a_boolean_property_schema_is_reported() {
        assert_eq!(
            paths(json!({ "type": "object", "properties": { "anything": true } })),
            vec!["/properties/anything"]
        );
    }

    #[test]
    fn the_root_and_non_schemas_are_not_reported() {
        assert!(paths(json!({})).is_empty());
        assert!(paths(json!({ "properties": { "a": { "type": "string" } } })).is_empty());
        assert!(paths(json!("not a schema")).is_empty());
        assert!(paths(json!({ "type": "object", "properties": { "odd": 7 } })).is_empty());
        assert!(paths(json!({ "type": "object", "properties": "not a map" })).is_empty());
    }

    #[test]
    fn property_names_are_escaped_as_pointer_tokens() {
        assert_eq!(
            paths(json!({
                "type": "object",
                "properties": { "a/b~c": { "enum": [1] } }
            })),
            vec!["/properties/a~1b~0c"]
        );
    }
}
