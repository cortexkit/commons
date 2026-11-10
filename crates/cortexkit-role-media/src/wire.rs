//! The body of a reply, as it rides a frame.
//!
//! Every `media-provider/v1` reply is a `RESPONSE` frame whose body is exactly
//! `{"result": <answer>}`: one object with the single key `result`, holding
//! the op's answer. A reader refuses a bare answer, and a body with any
//! top-level key beside `result`, by name ([`ReplyShapeError`]), rather than
//! guessing which shape it was sent. This includes terminal stream replies,
//! not `STREAM_DATA` events or referenced output-object contents.
//!
//! A refusal is an `ERROR` frame whose body is `{code, message, detail}`,
//! never wrapped in `result`.

use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

/// The one top-level key of a reply body.
pub const RESULT_KEY: &str = "result";

/// The body bytes of a reply carrying `answer`: `{"result": <answer>}`.
///
/// Fails only when `answer` cannot be encoded as JSON.
pub fn reply_body<T: Serialize + ?Sized>(answer: &T) -> serde_json::Result<Vec<u8>> {
    let mut body = Map::new();
    body.insert(RESULT_KEY.to_owned(), serde_json::to_value(answer)?);
    serde_json::to_vec(&Value::Object(body))
}

/// Decode a reply body: check that it is exactly `{"result": <answer>}`,
/// then decode the answer as `T`.
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
    use serde_json::json;

    #[test]
    fn reply_body_wraps_the_answer() {
        let answer = json!({"acked_through_seq": 7});
        let body = reply_body(&answer).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&body).unwrap(),
            json!({"result": {"acked_through_seq": 7}})
        );
        assert_eq!(read_reply::<Value>(&body), Ok(answer));
        assert_eq!(
            read_reply::<String>(&reply_body("words").unwrap()),
            Ok("words".into())
        );
    }

    #[test]
    fn bare_answers_are_refused_by_name() {
        for body in [
            br#"{"acked_through_seq":7}"#.as_slice(),
            br#""words""#,
            b"[]",
            b"null",
        ] {
            let error = read_reply::<Value>(body).unwrap_err();
            assert_eq!(error, ReplyShapeError::Bare);
            assert_eq!(error.name(), "bare_answer");
        }
    }

    #[test]
    fn extra_keys_are_refused_by_name_in_sorted_order() {
        let error = read_reply::<String>(br#"{"result":"words","id":1,"at":2}"#).unwrap_err();
        assert_eq!(
            error,
            ReplyShapeError::ExtraKeys(vec!["at".into(), "id".into()])
        );
        assert_eq!(error.name(), "extra_keys");
    }

    #[test]
    fn malformed_json_and_bad_answers_are_refused_by_name() {
        let error = read_reply::<String>(b"not json").unwrap_err();
        assert!(matches!(error, ReplyShapeError::NotJson(_)));
        assert_eq!(error.name(), "not_json");
        let error = read_reply::<String>(br#"{"result":7}"#).unwrap_err();
        assert!(matches!(error, ReplyShapeError::BadAnswer(_)));
        assert_eq!(error.name(), "bad_answer");
    }
}
