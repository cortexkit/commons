//! How every op travels between modules. A request body is the JSON object
//! `{"method": <op name>, "params": <the op's request>}`; a successful answer
//! is the op's answer as JSON. A refusal travels as the transport's error frame
//! `{code, message, detail?}` ([`Refusal`]): `code` is a stable name, the
//! message is for people and no program parses it, and machine-readable
//! fields go in `detail`.
//!
//! A frame the daemon itself produced (`frame_too_large`, a closed route)
//! carries the daemon-origin flag, so a receiver never mistakes it for the
//! peer's refusal.
//!
//! Model tool calls and the edit to a person's message before it is stored are
//! not part of this contract: they use the published `tool-provider/v1` and
//! `step-transform-provider` contracts, each on a route of its own.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize)]
struct Call<P> {
    method: String,
    params: P,
}

/// The body of a request for `method`.
pub fn encode<P: Serialize>(method: &str, params: &P) -> Vec<u8> {
    serde_json::to_vec(&Call {
        method: method.to_owned(),
        params,
    })
    .expect("op requests serialize")
}

/// Split a request body into its method and its still-encoded parameters.
pub fn decode(body: &[u8]) -> Result<(String, Value), Refusal> {
    let call: Call<Value> = serde_json::from_slice(body)
        .map_err(|error| Refusal::invalid_params("", format!("not an op request: {error}")))?;
    Ok((call.method, call.params))
}

/// Decode an op's parameters into its request type, naming the field that
/// failed (a path of dotted fields and bracketed array indexes, such as
/// `select.tags[0]`; empty for the whole object).
pub fn params<T: DeserializeOwned>(params: Value) -> Result<T, Refusal> {
    serde_path_to_error::deserialize(params).map_err(|error| {
        let field = error.path().to_string();
        let mut field = if field == "." { String::new() } else { field };
        let mut message = error.into_inner().to_string();
        if let Some(nested) = message.strip_prefix(FIELD_ERROR) {
            if let Ok(nested) = serde_json::from_str::<FieldError>(nested) {
                if !field.is_empty() && !nested.field.is_empty() && !nested.field.starts_with('[') {
                    field.push('.');
                }
                field.push_str(&nested.field);
                message = nested.message;
            }
        }
        Refusal::invalid_params(&field, message)
    })
}

const FIELD_ERROR: &str = "compaction field error: ";

#[derive(Serialize, Deserialize)]
struct FieldError {
    field: String,
    message: String,
}

// Tagged values are buffered to preserve unknown variants. Carry the field
// path across that buffer so invalid_params still names the nested field.
//
// Buffering starts a fresh path tracker, so the inner relative path and message
// travel to the enclosing `params` call in a reserved serde-error envelope
// (`FIELD_ERROR` followed by `{field, message}` JSON). `params` unwraps it and
// appends the relative path before building the wire refusal. This envelope is
// internal to decoding and never a format peers parse; only a whole error
// string that starts with the prefix and decodes as the envelope is treated as
// one.
pub(crate) fn tagged_fields<T: DeserializeOwned, E: serde::de::Error>(
    value: Value,
) -> Result<T, E> {
    params(value).map_err(|refusal| {
        let error = FieldError {
            field: refusal.detail.unwrap()["field"]
                .as_str()
                .unwrap()
                .to_owned(),
            message: refusal.message,
        };
        E::custom(format!(
            "{FIELD_ERROR}{}",
            serde_json::to_string(&error).unwrap()
        ))
    })
}

/// An op's refusal: the transport's error frame body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Refusal {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Value>,
}

impl Refusal {
    pub fn new(code: &str, message: impl Into<String>, detail: Option<Value>) -> Self {
        Self {
            code: code.to_owned(),
            message: message.into(),
            detail,
        }
    }

    /// `params` does not decode into the op's request. A bug in the caller:
    /// it logs it. A runner refused this on `compaction.setup` applies W1.3a;
    /// on other methods the caller does not retry.
    pub fn invalid_params(field: &str, message: impl Into<String>) -> Self {
        Self::new(
            codes::INVALID_PARAMS,
            message,
            Some(json!({ "field": field })),
        )
    }

    /// The route's role does not serve this method. A bug in the caller.
    pub fn unknown_method(method: &str) -> Self {
        Self::new(
            codes::UNKNOWN_METHOD,
            format!("this route does not serve `{method}`"),
            Some(json!({ "method": method })),
        )
    }

    /// Setup names a preset the provider does not know. The runner logs it and
    /// handles it as any refused Setup (W1.3a): a view already applied stays
    /// applied and Setup runs again at the next S1 trigger, while a session
    /// with no view ends the turn as `compaction_unavailable` before any model
    /// call.
    pub fn unknown_preset(preset: &str) -> Self {
        Self::new(
            codes::UNKNOWN_PRESET,
            format!("unknown preset `{preset}`"),
            Some(json!({ "preset": preset })),
        )
    }

    /// A ride for a session the provider holds no Setup for. The runner treats
    /// it as `nothing`, then calls Setup before its next notice.
    pub fn unknown_session() -> Self {
        Self::new(codes::UNKNOWN_SESSION, "no Setup for this session", None)
    }
}

/// Refusal codes. Each names why.
pub mod codes {
    pub const INVALID_PARAMS: &str = "invalid_params";
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    pub const UNKNOWN_SESSION: &str = "unknown_session";
    pub const UNKNOWN_PRESET: &str = "unknown_preset";
    /// Opening a route: the daemon attests no module for the caller.
    pub const UNATTESTED_RUNNER: &str = "unattested_runner";
    /// Opening a route: it declares none of the roles its receiver serves,
    /// or more than one.
    pub const UNKNOWN_ROUTE_ROLE: &str = "unknown_route_role";
    /// The route has no bound session. Refused at bind, except for
    /// step-transform-provider/v2, which refuses each unbound transform.hook
    /// call while answering role.describe and transform.declare on any route.
    pub const ROUTE_UNBOUND: &str = "route_unbound";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Request {
        preset: String,
        nested: Nested,
    }

    #[derive(Debug, Deserialize)]
    #[allow(dead_code)]
    struct Nested {
        count: u64,
    }

    #[test]
    fn a_bad_request_names_the_field_it_failed_at() {
        let refusal =
            params::<Request>(json!({"preset": "head", "nested": {"count": "x"}})).unwrap_err();
        assert_eq!(refusal.code, codes::INVALID_PARAMS);
        assert_eq!(refusal.detail, Some(json!({"field": "nested.count"})));
        let refusal = params::<Request>(json!({"nested": {"count": 1}})).unwrap_err();
        assert_eq!(refusal.detail, Some(json!({"field": ""})));
    }
}
