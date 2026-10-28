//! Structured output: a caller asks a run for a JSON value shaped by a JSON
//! Schema, and reads back what the run produced and whether it validated.
//!
//! The request rides inside `session.send` as `structured_output` and stays in
//! [`SendRequest::runner_params`](crate::send::SendRequest::runner_params);
//! read it with [`SendRequest::structured_output`](crate::send::SendRequest::structured_output).
//! The answer rides on `run.result` and `run.status` as
//! [`StructuredOutputResult`]. A runner serves either only when `role.describe`
//! lists the `structured_output` group.

use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// The `session.send` member that carries the request, and the prefix of
/// every `invalid_params` field path this module returns.
pub const FIELD: &str = "structured_output";

/// The tool the runner adds in `submit_tool` mode. Reserved: an admitted
/// tool of the same name is refused. Callers cannot choose another name.
pub const SUBMIT_RESULT_TOOL: &str = "submit_result";

/// The name used where a provider needs a schema name and the request gives
/// none.
pub const DEFAULT_NAME: &str = "result";

/// Longest accepted `name`, in bytes. A name is 1 to 48 of `[a-z0-9_]`.
pub const MAX_NAME_BYTES: usize = 48;

/// Largest accepted schema, measured as its RFC 8785 (JCS) encoding.
pub const MAX_SCHEMA_JCS_BYTES: usize = 65_536;

/// The only `$schema` value accepted when a schema names its dialect.
pub const DRAFT_2020_12: &str = "https://json-schema.org/draft/2020-12/schema";

/// At most this many entries in [`StructuredOutputResult::errors`].
pub const MAX_ERRORS: usize = 32;

/// At most this many bytes in one entry of [`StructuredOutputResult::errors`].
pub const MAX_ERROR_BYTES: usize = 256;

/// The two request modes, as they appear on the wire.
pub mod modes {
    /// Constrain the final assistant text. Refused for a run with tools.
    pub const FINAL_TEXT: &str = "final_text";
    /// Add the `submit_result` tool; calling it ends the run.
    pub const SUBMIT_TOOL: &str = "submit_tool";
}

/// Whether the provider was asked to guarantee the shape. A consumer decodes
/// an unknown value as a plain string rather than failing.
/// Decided at admission and fixed for the run, with its reason.
pub mod enforcement {
    pub const ENFORCED: &str = "enforced";
    pub const BEST_EFFORT: &str = "best_effort";
}

/// Why a run's enforcement is `best_effort`. A runner writes only these;
/// a consumer decodes the member as an open string.
pub mod reasons {
    /// The model catalog does not say whether the route supports schemas.
    pub const CATALOG_UNKNOWN: &str = "catalog_unknown";
    /// The catalog says the route does not support schemas.
    pub const ROUTE_UNSUPPORTED: &str = "route_unsupported";
    /// The schema uses a feature the route cannot enforce, judged at
    /// admission from the runner's own per-family subset profile. The runner
    /// sends such a schema without the provider's strict mode; it never
    /// refuses it, and never learns this from a provider error.
    pub const SCHEMA_OUTSIDE_SUBSET: &str = "schema_outside_subset";
    /// The route enforces schemas for one mode (final text or tool input)
    /// but not the requested one.
    pub const MODE_NOT_ENFORCEABLE: &str = "mode_not_enforceable";
    /// The runner's host provides no way to send the schema in a constrained
    /// form, regardless of what the route supports.
    pub const HOST_UNSUPPORTED: &str = "host_unsupported";
    /// Every reason a runner may write.
    pub const ALL: &[&str] = &[
        CATALOG_UNKNOWN,
        ROUTE_UNSUPPORTED,
        SCHEMA_OUTSIDE_SUBSET,
        MODE_NOT_ENFORCEABLE,
        HOST_UNSUPPORTED,
    ];
}

/// The runner's own validation of the value. A consumer decodes an unknown
/// value as a plain string rather than failing.
pub mod validation {
    /// `value` holds the parsed JSON, and it matches the schema.
    pub const PASSED: &str = "passed";
    /// `errors` lists what does not match; `value` holds what the model
    /// produced when it parsed as JSON.
    pub const FAILED: &str = "failed";
    /// No value was produced.
    pub const ABSENT: &str = "absent";
}

/// A structured-output request mode. Strict on the wire: an unknown mode is
/// refused naming `structured_output.mode`, never read as another mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Mode {
    FinalText,
    SubmitTool,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FinalText => modes::FINAL_TEXT,
            Self::SubmitTool => modes::SUBMIT_TOOL,
        }
    }
}

/// The `structured_output` member of `session.send`.
///
/// Non-exhaustive so later optional members are additive: build one with
/// [`StructuredOutput::new`] and [`StructuredOutput::with_name`], or read one
/// with [`SendRequest::structured_output`](crate::send::SendRequest::structured_output),
/// which checks every rule that does not depend on the session.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct StructuredOutput {
    pub mode: Mode,
    /// Where a provider needs a schema name; [`DEFAULT_NAME`] when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A JSON Schema (draft 2020-12) object.
    pub schema: Value,
}

impl StructuredOutput {
    pub fn new(mode: Mode, schema: Value) -> Self {
        Self {
            mode,
            schema,
            name: None,
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// The name a provider is given: `name`, or [`DEFAULT_NAME`].
    pub fn effective_name(&self) -> &str {
        self.name.as_deref().unwrap_or(DEFAULT_NAME)
    }

    /// The RFC 8785 (JCS) bytes of the schema: what a runner freezes on the
    /// run's start record and renders every request of the run from.
    pub fn schema_jcs(&self) -> Result<Vec<u8>, String> {
        serde_jcs::to_vec(&self.schema).map_err(|_| format!("{FIELD}.schema"))
    }

    /// Admission checks that depend on the runner and the session, after
    /// [`SendRequest::structured_output`](crate::send::SendRequest::structured_output)
    /// has checked the value itself. `advertised` is whether the runner's
    /// `role.describe` lists `structured_output`; `tools` is the run's
    /// admitted tool set, the plan's tools and the runner's own, without
    /// `submit_result`. Returns the field of the `invalid_params` refusal, in
    /// this order: the group is not advertised (`structured_output`), an
    /// admitted tool is named `submit_result` (`structured_output`), or
    /// `final_text` is asked for with a non-empty tool set
    /// (`structured_output.mode`).
    pub fn check<S: AsRef<str>>(&self, advertised: bool, tools: &[S]) -> Result<(), String> {
        if !advertised {
            return Err(FIELD.into());
        }
        if tools.iter().any(|tool| tool.as_ref() == SUBMIT_RESULT_TOOL) {
            return Err(FIELD.into());
        }
        if self.mode == Mode::FinalText && !tools.is_empty() {
            return Err(format!("{FIELD}.mode"));
        }
        Ok(())
    }
}

/// Decode and check the raw `structured_output` member of a send. Returns the
/// offending field path for an `invalid_params` refusal.
pub(crate) fn decode(raw: &Value) -> Result<StructuredOutput, String> {
    let members = raw.as_object().ok_or_else(|| FIELD.to_owned())?;
    if let Some(unknown) = members
        .keys()
        .find(|key| !matches!(key.as_str(), "mode" | "schema" | "name"))
    {
        return Err(format!("{FIELD}.{unknown}"));
    }
    if !matches!(
        members.get("mode").and_then(Value::as_str),
        Some(modes::FINAL_TEXT | modes::SUBMIT_TOOL)
    ) {
        return Err(format!("{FIELD}.mode"));
    }
    if let Some(name) = members.get("name") {
        if !name.as_str().is_some_and(name_well_formed) {
            return Err(format!("{FIELD}.name"));
        }
    }
    let schema_field = || format!("{FIELD}.schema");
    let schema = members
        .get("schema")
        .filter(|schema| schema.is_object())
        .ok_or_else(schema_field)?;
    check_schema(schema).map_err(|_| schema_field())?;
    let request: StructuredOutput =
        serde_json::from_value(raw.clone()).map_err(|_| FIELD.to_owned())?;
    if request.schema_jcs()?.len() > MAX_SCHEMA_JCS_BYTES {
        return Err(schema_field());
    }
    Ok(request)
}

/// Whether `name` is 1 to [`MAX_NAME_BYTES`] of `[a-z0-9_]`.
pub fn name_well_formed(name: &str) -> bool {
    (1..=MAX_NAME_BYTES).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Why a schema is refused before compilation. A runner refuses each as
/// `invalid_params` naming `structured_output.schema`.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SchemaProblem {
    /// A `$ref` or `$dynamicRef` points outside the schema document. Runners
    /// never fetch a schema over the network or from files.
    ExternalReference(String),
    /// `$schema` names a dialect other than draft 2020-12.
    OtherDialect(String),
}

/// Keywords whose values are instance data, not schemas: a `$ref` or `$id`
/// inside them is a plain value, never a reference.
const DATA_KEYWORDS: &[&str] = &["const", "enum", "default", "examples"];

/// Draft 2020-12 keywords whose value is one schema.
const SCHEMA_KEYWORDS: &[&str] = &[
    "items",
    "contains",
    "additionalProperties",
    "propertyNames",
    "if",
    "then",
    "else",
    "not",
    "unevaluatedItems",
    "unevaluatedProperties",
    "contentSchema",
];

/// Draft 2020-12 keywords whose value is an array of schemas.
const SCHEMA_ARRAY_KEYWORDS: &[&str] = &["allOf", "anyOf", "oneOf", "prefixItems"];

/// Draft 2020-12 keywords whose value maps names to schemas.
const SCHEMA_MAP_KEYWORDS: &[&str] = &[
    "properties",
    "patternProperties",
    "$defs",
    "dependentSchemas",
];

/// Where the walk is. In a known schema position `$id` declares a resource
/// and data keywords are skipped. Anywhere else (inside a keyword draft
/// 2020-12 does not define) nothing is skipped and no `$id` counts, so a
/// reference hidden there is still judged and cannot be made to look internal.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Root,
    Schema,
    Unknown,
}

/// Check the parts of `schema` this crate can judge without compiling it:
/// every `$ref` and `$dynamicRef` resolves inside the document, and
/// `$schema`, where present, is draft 2020-12.
///
/// A reference is inside the document when it starts with `#` (a JSON
/// Pointer or an anchor), or when its part before any `#` equals, exactly,
/// the `$id` of the root or of a subschema in the same document (with an
/// empty trailing `#` removed). Anything else is external, including a
/// relative reference that would only resolve against a base URI.
pub fn check_schema(schema: &Value) -> Result<(), SchemaProblem> {
    let mut ids = BTreeSet::new();
    let mut references = Vec::new();
    walk(schema, Position::Root, &mut ids, &mut references)?;
    for reference in references {
        let base = reference.split('#').next().unwrap_or_default();
        if !base.is_empty() && !ids.contains(base) {
            return Err(SchemaProblem::ExternalReference(reference));
        }
    }
    Ok(())
}

fn walk(
    value: &Value,
    position: Position,
    ids: &mut BTreeSet<String>,
    references: &mut Vec<String>,
) -> Result<(), SchemaProblem> {
    match value {
        Value::Object(members) => walk_object(members, position, ids, references),
        Value::Array(items) => {
            let position = match position {
                Position::Root => Position::Schema,
                other => other,
            };
            for item in items {
                walk(item, position, ids, references)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn walk_object(
    members: &Map<String, Value>,
    position: Position,
    ids: &mut BTreeSet<String>,
    references: &mut Vec<String>,
) -> Result<(), SchemaProblem> {
    for (key, value) in members {
        let keyword = key.as_str();
        if let ("$ref" | "$dynamicRef", Value::String(reference)) = (keyword, value) {
            references.push(reference.clone());
            continue;
        }
        if position == Position::Unknown {
            walk(value, Position::Unknown, ids, references)?;
            continue;
        }
        match (keyword, value) {
            ("$id", Value::String(id)) => {
                ids.insert(id.strip_suffix('#').unwrap_or(id).to_owned());
            }
            ("$schema", Value::String(dialect)) if position == Position::Root => {
                if dialect.strip_suffix('#').unwrap_or(dialect) != DRAFT_2020_12 {
                    return Err(SchemaProblem::OtherDialect(dialect.clone()));
                }
            }
            _ if DATA_KEYWORDS.contains(&keyword) => {}
            _ if SCHEMA_KEYWORDS.contains(&keyword) || SCHEMA_ARRAY_KEYWORDS.contains(&keyword) => {
                walk(value, Position::Schema, ids, references)?;
            }
            (_, Value::Object(map)) if SCHEMA_MAP_KEYWORDS.contains(&keyword) => {
                for schema in map.values() {
                    walk(schema, Position::Schema, ids, references)?;
                }
            }
            _ => walk(value, Position::Unknown, ids, references)?,
        }
    }
    Ok(())
}

/// The `structured_output` member of `run.result` and `run.status`, on an
/// ended run admitted with structured output. Decoded leniently: `mode`,
/// `enforcement`, `reason` and `validation` are open strings, and members a
/// later version adds are ignored. Members that do not apply are omitted,
/// never empty or zero.
///
/// Non-exhaustive so later optional members are additive: build one with
/// [`StructuredOutputResult::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct StructuredOutputResult {
    /// The request's mode, one of [`modes`].
    pub mode: String,
    /// `enforced` when the provider was asked to guarantee the shape, otherwise
    /// `best_effort`; see [`enforcement`].
    pub enforcement: String,
    /// On `best_effort` only: why no guarantee was asked for (the catalog does not
    /// say, the route has no support, the schema is outside the route's subset,
    /// or the route cannot enforce this mode); see [`reasons`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// One of [`validation`].
    pub validation: String,
    /// The value the model produced: the parsed JSON on `passed`, and on
    /// `failed` when it parsed as JSON. Never cut short; it counts toward the
    /// reply's frame limit (`reply_too_large`). A JSON `null` value is kept
    /// as `Some(Value::Null)`, distinct from an absent one.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub value: Option<Value>,
    /// On `failed`: messages prefixed with the JSON Pointer of the failing
    /// instance location, at most [`MAX_ERRORS`] of at most
    /// [`MAX_ERROR_BYTES`] bytes each.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<String>>,
    /// `true` when `errors` was cut to fit its bounds; omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub errors_truncated: Option<bool>,
    /// On `submit_tool`: the calls of the submitting step that were never
    /// dispatched, in the order the model emitted them; `[]` when
    /// `submit_result` came alone or was never called.
    /// With `submit_streamed_siblings`, an earlier call already dispatched
    /// while the step streamed keeps its own result and is not listed here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_siblings: Option<Vec<SkippedSibling>>,
}

/// Decode a member that is present as `Some`, keeping JSON `null`.
fn present<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// A call of the submitting step that the runner never dispatched.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct SkippedSibling {
    /// The model's id for the call.
    pub tool_call_id: String,
    /// The tool the call named.
    pub tool: String,
}

impl SkippedSibling {
    pub fn new(tool_call_id: impl Into<String>, tool: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            tool: tool.into(),
        }
    }
}

/// Names of the rules [`StructuredOutputResult::well_formed`] checks.
pub mod result_problems {
    pub const REASON_WITHOUT_BEST_EFFORT: &str = "reason_without_best_effort";
    pub const BEST_EFFORT_WITHOUT_REASON: &str = "best_effort_without_reason";
    pub const PASSED_WITHOUT_VALUE: &str = "passed_without_value";
    pub const VALUE_WHEN_ABSENT: &str = "value_when_absent";
    pub const ERRORS_WITHOUT_FAILED: &str = "errors_without_failed";
    pub const FAILED_WITHOUT_ERRORS: &str = "failed_without_errors";
    pub const TOO_MANY_ERRORS: &str = "too_many_errors";
    pub const ERROR_TOO_LONG: &str = "error_too_long";
    pub const ERRORS_TRUNCATED_NOT_TRUE: &str = "errors_truncated_not_true";
    pub const ERRORS_TRUNCATED_WITHOUT_ERRORS: &str = "errors_truncated_without_errors";
    pub const SUBMIT_TOOL_WITHOUT_SKIPPED_SIBLINGS: &str = "submit_tool_without_skipped_siblings";
    pub const SKIPPED_SIBLINGS_WITHOUT_SUBMIT_TOOL: &str = "skipped_siblings_without_submit_tool";
}

impl StructuredOutputResult {
    /// A result with no optional member. `submit_tool` results also need
    /// [`Self::with_skipped_siblings`], even when empty.
    pub fn new(
        mode: impl Into<String>,
        enforcement: impl Into<String>,
        validation: impl Into<String>,
    ) -> Self {
        Self {
            mode: mode.into(),
            enforcement: enforcement.into(),
            reason: None,
            validation: validation.into(),
            value: None,
            errors: None,
            errors_truncated: None,
            skipped_siblings: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_value(mut self, value: Value) -> Self {
        self.value = Some(value);
        self
    }

    /// Set `errors`, cut to their bounds by [`bound_errors`], and
    /// `errors_truncated` when anything was cut.
    pub fn with_errors(mut self, errors: Vec<String>) -> Self {
        let (errors, truncated) = bound_errors(errors);
        self.errors = Some(errors);
        self.errors_truncated = truncated.then_some(true);
        self
    }

    pub fn with_skipped_siblings(mut self, skipped: Vec<SkippedSibling>) -> Self {
        self.skipped_siblings = Some(skipped);
        self
    }

    /// Check the member rules for the values this crate knows, returning the
    /// name of the first broken rule ([`result_problems`]). Unknown modes,
    /// enforcements and validations are not constrained. A consumer treats a
    /// result that breaks a rule as malformed; it is not a refusal.
    pub fn well_formed(&self) -> Result<(), &'static str> {
        use result_problems as p;
        match (self.enforcement.as_str(), &self.reason) {
            (enforcement::ENFORCED, Some(_)) => return Err(p::REASON_WITHOUT_BEST_EFFORT),
            (enforcement::BEST_EFFORT, None) => return Err(p::BEST_EFFORT_WITHOUT_REASON),
            _ => {}
        }
        match self.validation.as_str() {
            validation::PASSED if self.value.is_none() => return Err(p::PASSED_WITHOUT_VALUE),
            validation::ABSENT if self.value.is_some() => return Err(p::VALUE_WHEN_ABSENT),
            validation::FAILED if self.errors.as_ref().is_none_or(Vec::is_empty) => {
                return Err(p::FAILED_WITHOUT_ERRORS)
            }
            validation::PASSED | validation::ABSENT if self.errors.is_some() => {
                return Err(p::ERRORS_WITHOUT_FAILED)
            }
            _ => {}
        }
        if let Some(errors) = &self.errors {
            if errors.len() > MAX_ERRORS {
                return Err(p::TOO_MANY_ERRORS);
            }
            if errors.iter().any(|error| error.len() > MAX_ERROR_BYTES) {
                return Err(p::ERROR_TOO_LONG);
            }
        }
        match (self.errors_truncated, &self.errors) {
            (Some(false), _) => return Err(p::ERRORS_TRUNCATED_NOT_TRUE),
            (Some(true), None) => return Err(p::ERRORS_TRUNCATED_WITHOUT_ERRORS),
            _ => {}
        }
        match (self.mode.as_str(), &self.skipped_siblings) {
            (modes::SUBMIT_TOOL, None) => Err(p::SUBMIT_TOOL_WITHOUT_SKIPPED_SIBLINGS),
            (modes::FINAL_TEXT, Some(_)) => Err(p::SKIPPED_SIBLINGS_WITHOUT_SUBMIT_TOOL),
            _ => Ok(()),
        }
    }
}

/// Cut validation messages to at most [`MAX_ERRORS`] entries of at most
/// [`MAX_ERROR_BYTES`] bytes each, shortening an entry at a UTF-8 character
/// boundary. The flag is `true` when an entry was dropped or shortened.
pub fn bound_errors(errors: Vec<String>) -> (Vec<String>, bool) {
    let mut truncated = errors.len() > MAX_ERRORS;
    let bounded = errors
        .into_iter()
        .take(MAX_ERRORS)
        .map(|mut error| {
            if error.len() > MAX_ERROR_BYTES {
                let mut end = MAX_ERROR_BYTES;
                while !error.is_char_boundary(end) {
                    end -= 1;
                }
                error.truncate(end);
                truncated = true;
            }
            error
        })
        .collect();
    (bounded, truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        describe::{check_describe, DescribeProblem, RoleDescribe},
        errors::{self, ReplyTooLargeDetail, ReplyTooLargeSubject},
        run::{RunResult, RunStatus},
        send::SendRequest,
        vectors,
    };
    use serde_json::json;

    /// The send params of a vector case, with any `pad_to_jcs_bytes` applied.
    fn params(case: &Value) -> Value {
        let mut params = case["params"].clone();
        if let Some(target) = case["pad_to_jcs_bytes"].as_u64() {
            let schema = &mut params["structured_output"]["schema"];
            let base = serde_jcs::to_vec(schema).unwrap().len() as u64;
            schema["description"] = json!("a".repeat((target - base) as usize));
            assert_eq!(serde_jcs::to_vec(schema).unwrap().len() as u64, target);
        }
        params
    }

    fn tools(case: &Value) -> Vec<String> {
        serde_json::from_value(case["tools"].clone()).unwrap()
    }

    #[test]
    fn structured_output_stays_in_runner_params_byte_for_byte() {
        let file = vectors::load("structured-output.json");
        let case = &file["preserved"];
        let expected = case["structured_output_bytes"].as_str().unwrap().as_bytes();
        let request: SendRequest = vectors::round_trip("preserved", &case["params"]);
        // Consumers that predate the typed accessor read this exact map entry.
        assert_eq!(
            serde_json::to_vec(&request.runner_params["structured_output"]).unwrap(),
            expected
        );
        let typed = request.structured_output().unwrap().unwrap();
        assert_eq!(serde_json::to_vec(&typed).unwrap(), expected);
        let rebuilt = SendRequest::new("start", "s1").with_structured_output(typed);
        assert_eq!(rebuilt, request);
        let encoded = serde_json::to_string(&rebuilt).unwrap();
        assert_eq!(encoded.matches("\"structured_output\":").count(), 1);
        assert_eq!(SendRequest::new("p", "k").structured_output(), Ok(None));
    }

    #[test]
    fn structured_output_send_vectors_accept_each_mode() {
        let file = vectors::load("structured-output.json");
        let mut seen = BTreeSet::new();
        for case in vectors::cases(&file, "accept") {
            let name = case["name"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &params(case));
            let typed = request
                .structured_output()
                .unwrap_or_else(|field| panic!("{name}: refused {field}"))
                .unwrap();
            typed
                .check(case["advertised"].as_bool().unwrap(), &tools(case))
                .unwrap_or_else(|field| panic!("{name}: refused {field}"));
            seen.insert(typed.mode.as_str());
            if let Some(jcs) = case["schema_jcs"].as_str() {
                assert_eq!(typed.schema_jcs().unwrap(), jcs.as_bytes(), "{name}");
            }
            if let Some(effective) = case["effective_name"].as_str() {
                assert_eq!(typed.effective_name(), effective, "{name}");
            }
        }
        assert_eq!(seen.len(), 2, "both modes are accepted");
    }

    #[test]
    fn structured_output_send_vectors_refuse_by_field() {
        let file = vectors::load("structured-output.json");
        let refusals = vectors::cases(&file, "refuse");
        for case in refusals {
            let name = case["name"].as_str().unwrap();
            assert_eq!(case["error"]["code"], errors::INVALID_PARAMS);
            let field = case["error"]["detail"]["field"].as_str().unwrap();
            let request: SendRequest = vectors::round_trip(name, &params(case));
            let result = request.structured_output().and_then(|typed| {
                typed
                    .unwrap()
                    .check(case["advertised"].as_bool().unwrap(), &tools(case))
            });
            assert_eq!(result, Err(field.to_owned()), "{name}");
        }
        for required in [
            "final_text with tools",
            "reserved-name collision",
            "external absolute $ref",
            "schema one byte over the cap",
            "uppercase name",
        ] {
            assert!(
                refusals.iter().any(|case| case["name"] == required),
                "missing refusal {required}"
            );
        }
    }

    #[test]
    fn external_references_are_refused_by_name() {
        assert_eq!(
            check_schema(&json!({"$ref": "https://example.com/s.json"})),
            Err(SchemaProblem::ExternalReference(
                "https://example.com/s.json".into()
            ))
        );
        assert_eq!(
            check_schema(&json!({"$dynamicRef": "meta.json#m"})),
            Err(SchemaProblem::ExternalReference("meta.json#m".into()))
        );
        assert_eq!(
            check_schema(&json!({"$schema": "http://json-schema.org/draft-07/schema#"})),
            Err(SchemaProblem::OtherDialect(
                "http://json-schema.org/draft-07/schema#".into()
            ))
        );
        assert_eq!(
            check_schema(&json!({"$schema": DRAFT_2020_12, "$ref": "#/$defs/a"})),
            Ok(())
        );
    }

    #[test]
    fn structured_output_describe_group_requires_send() {
        let file = vectors::load("structured-output-describe.json");
        let answer: RoleDescribe = vectors::round_trip("describe", &file["accept"]);
        assert_eq!(check_describe(&file["accept"]).unwrap(), answer);
        assert!(answer.declares(crate::capabilities::STRUCTURED_OUTPUT));
        assert_eq!(
            check_describe(&file["refuse"]),
            Err(vec![DescribeProblem::GroupIncomplete {
                capability: crate::capabilities::STRUCTURED_OUTPUT,
                op: crate::ops::SESSION_SEND
            }])
        );
    }

    #[test]
    fn structured_output_result_vectors_round_trip_and_agree() {
        let file = vectors::load("structured-output-results.json");
        let mut reasons_seen = BTreeSet::new();
        let mut validations_seen = BTreeSet::new();
        for case in vectors::cases(&file, "canonical") {
            let name = case["name"].as_str().unwrap();
            let result: RunResult = vectors::round_trip(name, &case["result"]);
            let status: RunStatus = vectors::round_trip(name, &case["status"]);
            assert_eq!(result.structured_output, status.structured_output, "{name}");
            assert!(result.final_message_consistent(), "{name}");
            let output = result.structured_output.unwrap();
            assert_eq!(output.well_formed(), Ok(()), "{name}");
            if let Some(reason) = &output.reason {
                assert!(reasons::ALL.contains(&reason.as_str()), "{name}");
                reasons_seen.insert(reason.clone());
            }
            validations_seen.insert(output.validation.clone());
            if output.errors_truncated == Some(true) {
                assert_eq!(output.errors.as_ref().unwrap().len(), MAX_ERRORS, "{name}");
            }
        }
        assert_eq!(reasons_seen.len(), reasons::ALL.len());
        assert_eq!(validations_seen.len(), 3);
    }

    #[test]
    fn malformed_structured_output_results_name_the_broken_rule() {
        let file = vectors::load("structured-output-results.json");
        for case in vectors::cases(&file, "malformed") {
            let name = case["name"].as_str().unwrap();
            let output: StructuredOutputResult =
                vectors::round_trip(name, &case["structured_output"]);
            assert_eq!(
                output.well_formed(),
                Err(case["problem"].as_str().unwrap()),
                "{name}"
            );
        }
        let open: StructuredOutputResult =
            vectors::round_trip("open", &file["open"]["structured_output"]);
        assert_eq!(open.well_formed(), Ok(()));
    }

    #[test]
    fn host_unsupported_round_trips_and_is_well_formed() {
        let output = StructuredOutputResult::new(
            modes::SUBMIT_TOOL,
            enforcement::BEST_EFFORT,
            validation::PASSED,
        )
        .with_reason(reasons::HOST_UNSUPPORTED)
        .with_value(json!({"answer": "42"}))
        .with_skipped_siblings(vec![]);
        let wire = serde_json::to_value(&output).unwrap();
        assert_eq!(wire["reason"], "host_unsupported");
        let decoded: StructuredOutputResult = serde_json::from_value(wire).unwrap();
        assert_eq!(decoded, output);
        assert_eq!(decoded.well_formed(), Ok(()));
    }

    #[test]
    fn errors_are_bounded_and_marked_when_cut() {
        let fits = StructuredOutputResult::new(modes::FINAL_TEXT, enforcement::ENFORCED, "failed")
            .with_errors(vec!["/a: expected string".into()]);
        assert_eq!(fits.errors_truncated, None);
        let many: Vec<String> = (0..40).map(|i| format!("/{i}: bad")).collect();
        let cut = fits.clone().with_errors(many);
        assert_eq!(cut.errors.as_ref().unwrap().len(), MAX_ERRORS);
        assert_eq!(cut.errors_truncated, Some(true));
        assert_eq!(cut.well_formed(), Ok(()));
        // A multi-byte character straddling the limit is dropped whole.
        let long = format!("/a: {}", "é".repeat(200));
        let (bounded, truncated) = bound_errors(vec![long]);
        assert!(truncated);
        assert!(bounded[0].len() <= MAX_ERROR_BYTES && bounded[0].len() >= MAX_ERROR_BYTES - 1);
    }

    #[test]
    fn an_oversized_run_answer_is_refused_naming_the_run() {
        let file = vectors::load("structured-output-results.json");
        let refusal = &file["reply_too_large"];
        assert_eq!(refusal["code"], errors::REPLY_TOO_LARGE);
        let detail: ReplyTooLargeDetail =
            vectors::round_trip("reply_too_large", &refusal["detail"]);
        assert_eq!(
            detail.subject,
            ReplyTooLargeSubject::Run {
                run_id: "r1".into()
            }
        );
        assert!(detail.bytes > detail.limit);
    }
}
