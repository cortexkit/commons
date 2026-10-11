//! Withdraw a submission before it starts, under the session owner's scope.

use serde::{Deserialize, Serialize};

/// The id returned in a pending send reply on a runner declaring `retract`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RetractRequest {
    pub submission_id: String,
}

impl RetractRequest {
    pub fn new(submission_id: impl Into<String>) -> Self {
        Self {
            submission_id: submission_id.into(),
        }
    }
}

/// A JSON string. Only `Retracted` confirms a durable withdrawal; unknown
/// replies survive decoding but never confirm a retraction.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum RetractReply {
    Retracted,
    AlreadyStarted,
    NotPending,
    Unknown(String),
}

impl From<String> for RetractReply {
    fn from(value: String) -> Self {
        match value.as_str() {
            "retracted" => Self::Retracted,
            "already_started" => Self::AlreadyStarted,
            "not_pending" => Self::NotPending,
            _ => Self::Unknown(value),
        }
    }
}

impl From<RetractReply> for String {
    fn from(value: RetractReply) -> Self {
        match value {
            RetractReply::Retracted => "retracted".into(),
            RetractReply::AlreadyStarted => "already_started".into(),
            RetractReply::NotPending => "not_pending".into(),
            RetractReply::Unknown(s) => s,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retract_wire_bytes_and_open_replies() {
        let request = RetractRequest::new("submission-1");
        assert_eq!(
            serde_json::to_string(&request).unwrap(),
            r#"{"submission_id":"submission-1"}"#
        );
        assert_eq!(
            serde_json::from_str::<RetractRequest>(
                r#"{"submission_id":"submission-1","future":true}"#
            )
            .unwrap(),
            request
        );
        assert!(serde_json::from_str::<RetractRequest>("{}").is_err());
        for (bytes, expected) in [
            (r#""retracted""#, RetractReply::Retracted),
            (r#""already_started""#, RetractReply::AlreadyStarted),
            (r#""not_pending""#, RetractReply::NotPending),
            (r#""future""#, RetractReply::Unknown("future".into())),
        ] {
            assert_eq!(
                serde_json::from_str::<RetractReply>(bytes).unwrap(),
                expected
            );
            assert_eq!(serde_json::to_string(&expected).unwrap(), bytes);
        }
    }
}
