//! `tool.catalog`: the catalog-and-text fetch.
//!
//! One op serves preflight's tool section, the runner's fetch of the
//! session's plan item and the live view. The answer is a pure function of
//! its inputs (the plan item's preset and params, the composition, the
//! provider's user and project configuration and host facts); the scope,
//! owner and agent on the route are attribution only and never change it.
//! Tools the project disables are omitted. Tool names and argument schemas
//! never depend on the composition; descriptions and system text may.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The result operations a hook may apply to a tool's result. A tool that
/// does not list `result_ops` accepts all three.
pub const RESULT_OPS_ALL: &[&str] = &["prepend", "append", "replace"];

/// Session-level capabilities a provider may report in its catalog answer
/// for a session. The runner freezes them with the tools it fetched.
pub mod session_capabilities {
    /// Parameters marked host-only exist, and a host may supply them.
    pub const HOST_PARAMS: &str = "host_params";
    /// The provider may settle a call later and serves `late_results`.
    pub const LATE_RESULTS: &str = "late_results";
}

/// The schema keyword marking a host-only parameter, and its value. Such a
/// parameter is always optional and is refused in a call from the model.
pub const AUDIENCE_KEY: &str = "x-ck-audience";
pub const AUDIENCE_HOST: &str = "host";

/// Root-level schema keywords a flat argument schema must not carry.
pub const ROOT_UNION_KEYWORDS: &[&str] = &["anyOf", "oneOf", "allOf"];

/// The arguments of a `tool.catalog` request.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct CatalogRequest {
    /// The plan item's params: the shared vocabulary (`behavior`, `scope`,
    /// `tool_descs`, `exclude`) plus any axes of the provider's own. A value
    /// the provider does not know is refused, never guessed.
    #[serde(default)]
    pub params: Map<String, Value>,
    /// The session's composition, carried verbatim as an opaque JSON object.
    /// Providers never interpret it beyond resolving their own text against
    /// it. Absent on a preflight call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition: Option<Map<String, Value>>,
    /// The provider's system-prompt item, when the plan has one for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_text: Option<SystemTextItem>,
    /// Ask for `{generation, catalog_digest}` only, for a cheap change check.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest_only: Option<bool>,
}

/// A system-prompt plan item fetched alongside the catalog.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SystemTextItem {
    pub preset: String,
    #[serde(default)]
    pub params: Map<String, Value>,
}

/// The `tool.catalog` answer. A `digest_only` answer carries only
/// `generation` and `catalog_digest`.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct CatalogAnswer {
    /// Opaque; changes whenever the catalog's content changes. A call's
    /// schema pin names the generation the runner froze.
    pub generation: String,
    /// The digest of the catalog's content under these inputs. Present in a
    /// `digest_only` answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_digest: Option<String>,
    /// The composition the answer was resolved against; absent on preflight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_digest: Option<String>,
    /// Every tool the provider serves under these inputs. A provider with only
    /// system text, and every `digest_only` answer, has none.
    #[serde(default)]
    pub tools: Vec<CatalogTool>,
    /// The system-prompt item's answer, when the request named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_text: Option<SystemTextAnswer>,
    /// Session-level capabilities, keyed by name ([`session_capabilities`]),
    /// each declared with the value `true`.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub capabilities: Map<String, Value>,
}

impl CatalogAnswer {
    /// Whether the answer declares the session capability `name`.
    pub fn declares(&self, name: &str) -> bool {
        self.capabilities.get(name) == Some(&Value::Bool(true))
    }
}

/// One tool in a catalog answer.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct CatalogTool {
    /// The provider's exact tool name, which is also how the user disables it.
    pub name: String,
    /// The digest of the argument schema. Opaque: compared for equality.
    pub schema_digest: String,
    /// The tool's behaviour version, bumped when its behaviour changes without
    /// a schema change. A call's schema pin names it.
    pub semantics: u64,
    /// The result operations the tool accepts from hooks; `None` means all
    /// of [`RESULT_OPS_ALL`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_ops: Option<Vec<String>>,
    /// Capability tags naming what the tool does, for example
    /// `code.outline/v1` or `acme:code.callgraph/v1`. Data only; never in the
    /// module's HELLO manifest.
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The argument schema, which must be flat (see [`check_flat_schema`]).
    pub input_schema: Value,
}

impl CatalogTool {
    pub fn effective_result_ops(&self) -> Vec<&str> {
        match &self.result_ops {
            Some(ops) => ops.iter().map(String::as_str).collect(),
            None => RESULT_OPS_ALL.to_vec(),
        }
    }
}

/// The system-prompt item's answer, from the same configuration resolution
/// as the catalog in the same reply.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct SystemTextAnswer {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    pub item_digest: String,
    pub preflight_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub composition_digest: Option<String>,
}

/// Why an argument schema is not flat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FlatnessProblem {
    /// The schema is not a JSON object.
    NotAnObject,
    /// The schema's root carries a union keyword.
    RootUnion { keyword: &'static str },
    /// The schema's root `type` is an array, which is a union of types.
    RootTypeArray,
}

/// Check an argument schema is flat: an object with no root-level `anyOf`,
/// `oneOf` or `allOf`, and no root-level `type` array. Unions below the root
/// are allowed.
pub fn check_flat_schema(schema: &Value) -> Result<(), FlatnessProblem> {
    let Some(object) = schema.as_object() else {
        return Err(FlatnessProblem::NotAnObject);
    };
    for keyword in ROOT_UNION_KEYWORDS {
        if object.contains_key(*keyword) {
            return Err(FlatnessProblem::RootUnion { keyword });
        }
    }
    if object.get("type").is_some_and(Value::is_array) {
        return Err(FlatnessProblem::RootTypeArray);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::vectors;

    #[test]
    fn schema_vectors_check_as_recorded() {
        let vectors = vectors::load("catalog-schemas.json");
        for case in vectors["flat"].as_array().unwrap() {
            assert_eq!(
                check_flat_schema(&case["schema"]),
                Ok(()),
                "{}",
                case["name"]
            );
        }
        for case in vectors["not_flat"].as_array().unwrap() {
            assert!(
                check_flat_schema(&case["schema"]).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn catalog_vectors_decode_as_recorded() {
        let vectors = vectors::load("catalog-answers.json");
        let full: CatalogAnswer = serde_json::from_value(vectors["full"].clone()).unwrap();
        assert_eq!(full.tools[0].effective_result_ops(), ["prepend", "append"]);
        assert_eq!(full.tools[1].effective_result_ops(), RESULT_OPS_ALL);
        assert_eq!(full.tools[1].semantics, 3);
        assert!(full.declares(session_capabilities::LATE_RESULTS));
        assert!(!full.declares(session_capabilities::HOST_PARAMS));

        let digest: CatalogAnswer = serde_json::from_value(vectors["digest_only"].clone()).unwrap();
        assert!(digest.tools.is_empty());
        assert_eq!(digest.generation, full.generation);
        assert!(digest.catalog_digest.is_some());

        for case in vectors["refused_tools"].as_array().unwrap() {
            assert!(
                serde_json::from_value::<CatalogTool>(case["tool"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
    }

    #[test]
    fn a_request_carries_the_composition_verbatim() {
        let request: CatalogRequest = serde_json::from_value(json!({
            "params": {"preset": "main"},
            "composition": {"anything": [1, {"nested": true}]},
            "digest_only": true
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(&request).unwrap()["composition"],
            json!({"anything": [1, {"nested": true}]})
        );
    }
}
