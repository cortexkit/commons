//! The body of a role reply, as it rides a frame.
//!
//! Every `llm-runner/v1` role reply is a `RESPONSE` frame whose body is
//! exactly `{"result": <answer>}`: one object with the single key `result`,
//! holding the op's answer (a `SendReply`, a `BaselineReply`, the string
//! `"ack"`, and so on). A reader refuses a bare answer, and a body with any
//! top-level key beside `result`, by name ([`ReplyShapeError`]), rather than
//! guessing which shape it was sent.
//!
//! Two other frames are not wrapped:
//!
//! - a `session.subscribe` `STREAM_DATA` frame carries the bare event
//!   ([`crate::subscribe::SubscribeEvent`]);
//! - a refusal is an `ERROR` frame whose body is `{code, message, detail?}`.
//!   Every refusal field other than `code` and `message` (a `class`, a
//!   `field`, a typed detail such as [`crate::errors::ReplyTooLargeDetail`])
//!   lives inside `detail`, never at the top level of the body.
//!
//! A run error ([`crate::run::RunError`] in `RunResult::error`) is part of
//! an answer, not a refusal: it keeps `class` at its own top level.

use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

/// The one top-level key of a reply body.
pub const RESULT_KEY: &str = "result";

/// The body bytes of a reply carrying `answer`: `{"result": <answer>}`.
///
/// Fails only when `answer` cannot be encoded as JSON (a map with non-string
/// keys, say).
pub fn reply_body<T: Serialize + ?Sized>(answer: &T) -> serde_json::Result<Vec<u8>> {
    let mut body = Map::new();
    body.insert(RESULT_KEY.to_owned(), serde_json::to_value(answer)?);
    serde_json::to_vec(&Value::Object(body))
}

/// Decode a reply body: check that it is exactly `{"result": <answer>}`,
/// then decode the answer as `T` with `T`'s own rules (lenient for the
/// replies this crate defines).
pub fn read_reply<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, ReplyShapeError> {
    let body: Value =
        serde_json::from_slice(bytes).map_err(|e| ReplyShapeError::NotJson(e.to_string()))?;
    let Value::Object(mut body) = body else {
        return Err(ReplyShapeError::Bare);
    };
    let Some(answer) = body.remove(RESULT_KEY) else {
        return Err(ReplyShapeError::Bare);
    };
    if !body.is_empty() {
        return Err(ReplyShapeError::ExtraKeys(body.keys().cloned().collect()));
    }
    serde_json::from_value(answer).map_err(|e| ReplyShapeError::BadAnswer(e.to_string()))
}

/// Why a reply body was refused by [`read_reply`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReplyShapeError {
    /// The body is not JSON at all.
    NotJson(String),
    /// The body is a bare answer: not an object holding `result`.
    Bare,
    /// The body holds `result` and these other top-level keys, in sorted
    /// order.
    ExtraKeys(Vec<String>),
    /// The body is wrapped correctly, but the answer inside `result` does
    /// not decode as the expected type.
    BadAnswer(String),
}

impl ReplyShapeError {
    /// A stable name for the problem, for a refusal message or a test.
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotJson(_) => "not_json",
            Self::Bare => "bare_answer",
            Self::ExtraKeys(_) => "extra_keys",
            Self::BadAnswer(_) => "bad_answer",
        }
    }
}

impl std::fmt::Display for ReplyShapeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotJson(error) => write!(f, "the reply body is not JSON: {error}"),
            Self::Bare => write!(
                f,
                "the reply body is a bare answer, not {{\"{RESULT_KEY}\": <answer>}}"
            ),
            Self::ExtraKeys(keys) => write!(
                f,
                "the reply body has top-level keys beside \"{RESULT_KEY}\": {keys:?}"
            ),
            Self::BadAnswer(error) => {
                write!(
                    f,
                    "the answer inside \"{RESULT_KEY}\" does not decode: {error}"
                )
            }
        }
    }
}

impl std::error::Error for ReplyShapeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{baseline::BaselineReply, errors, vectors};

    /// Decode `body` as the answer type named by `answer`, as a consumer of
    /// that op would.
    fn read_named(answer: &str, body: &[u8]) -> Result<Value, ReplyShapeError> {
        match answer {
            "BaselineReply" => {
                read_reply::<BaselineReply>(body).map(|reply| serde_json::to_value(reply).unwrap())
            }
            "string" => read_reply::<String>(body).map(Value::String),
            other => panic!("no answer type {other}"),
        }
    }

    #[test]
    fn reply_envelope_vectors_read_as_recorded() {
        let file = vectors::load("reply-envelope.json");
        let mut refused = Vec::new();
        for case in vectors::cases(&file, "replies") {
            let name = case["name"].as_str().unwrap();
            let body = serde_json::to_vec(&case["body"]).unwrap();
            let read = read_named(case["answer"].as_str().unwrap(), &body);
            match case.get("refused") {
                None => {
                    let answer = read.unwrap_or_else(|e| panic!("{name}: {e}"));
                    assert_eq!(answer, case["body"][RESULT_KEY], "{name}");
                    // Wrapping the decoded answer again gives the same body.
                    assert_eq!(
                        serde_json::from_slice::<Value>(&reply_body(&answer).unwrap()).unwrap(),
                        case["body"],
                        "{name}"
                    );
                }
                Some(refusal) => {
                    let error = read.expect_err(name);
                    assert_eq!(error.name(), refusal["problem"], "{name}");
                    if let Some(keys) = refusal.get("keys") {
                        assert_eq!(
                            error,
                            ReplyShapeError::ExtraKeys(
                                serde_json::from_value(keys.clone()).unwrap()
                            ),
                            "{name}"
                        );
                    }
                    refused.push(error.name());
                }
            }
        }
        for problem in ["bare_answer", "extra_keys", "bad_answer"] {
            assert!(refused.contains(&problem), "no vector refuses {problem}");
        }
        for case in vectors::cases(&file, "refusals") {
            let name = case["name"].as_str().unwrap();
            let body = case["body"].as_object().unwrap();
            let mut keys: Vec<&str> = body.keys().map(String::as_str).collect();
            keys.sort_unstable();
            assert!(
                keys == ["code", "detail", "message"] || keys == ["code", "message"],
                "{name}: a refusal body has only code, message and detail: {keys:?}"
            );
            assert!(
                errors::CODES.contains(&body["code"].as_str().unwrap()),
                "{name}"
            );
        }
        for case in vectors::cases(&file, "stream_events") {
            let name = case["name"].as_str().unwrap();
            let event: crate::subscribe::SubscribeEvent =
                serde_json::from_value(case["frame"].clone()).unwrap_or_else(|e| {
                    panic!("{name}: a stream frame carries the bare event: {e}")
                });
            assert!(case["frame"].get(RESULT_KEY).is_none(), "{name}");
            assert!(!event.kind.is_empty(), "{name}");
        }
    }

    #[test]
    fn a_bare_answer_and_extra_keys_are_refused_by_name() {
        assert_eq!(
            read_reply::<String>(br#""ack""#),
            Err(ReplyShapeError::Bare)
        );
        assert_eq!(
            read_reply::<String>(br#"{"result":"ack","id":1,"at":2}"#),
            Err(ReplyShapeError::ExtraKeys(vec!["at".into(), "id".into()]))
        );
        assert_eq!(
            read_reply::<String>(br#"{"result":"ack"}"#),
            Ok("ack".into())
        );
        assert_eq!(
            read_reply::<String>(b"not json").unwrap_err().name(),
            "not_json"
        );
        assert_eq!(reply_body("ack").unwrap(), br#"{"result":"ack"}"#);
    }
}
