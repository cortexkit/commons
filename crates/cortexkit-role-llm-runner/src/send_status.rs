//! The `send_status` group: `send.status`, what a re-send of a `send_id`
//! would answer, without re-sending the send's body.
//!
//! A runner that declares `send_status`
//! ([`crate::capabilities::SEND_STATUS`]) answers `send.status {send_id}`
//! from its durable record of that send, with the [`SendReply`] a re-send
//! of the same body would get, except that `send.status` never acts. A run
//! paused for re-authentication stays paused, reporting `pause_reason`;
//! only the caller's re-send may re-drive it. It writes and delivers nothing. A `send_id` the session
//! has no record of, for the caller, is refused
//! [`crate::errors::UNKNOWN_SEND`].
//!
//! A caller sends `send.status` only to a runner that declares the group.
//! Against any other runner it follows a send by re-sending the full body
//! under the same `send_id` (CONTRACT §9).

use serde::{Deserialize, Serialize};

pub use crate::send::SendReply;

/// The `field` an `invalid_params` refusal names for a missing or malformed
/// `send_id`.
pub const SEND_ID_FIELD: &str = "send_id";

/// The `send.status` request: `{send_id}`.
///
/// Strict: an unknown field is refused `invalid_params` naming it, because
/// a misspelled `send_id` would otherwise ask about another send, or about
/// none.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`SendStatusRequest::new`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct SendStatusRequest {
    /// The `send_id` of an earlier `session.send` by the same caller, in
    /// the same namespace a re-send uses: the caller and the `send_id`.
    pub send_id: String,
}

impl SendStatusRequest {
    pub fn new(send_id: impl Into<String>) -> Self {
        Self {
            send_id: send_id.into(),
        }
    }
}

/// The `send.status` answer: the [`SendReply`] a re-send of the same
/// `send_id` would get, except that it never acts (in particular, it never
/// resumes a paused run). Decoded leniently, as that reply is.
pub type SendStatusReply = SendReply;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors,
        send::{delivered_as, states, SendState},
        vectors,
    };

    #[test]
    fn send_status_requests_round_trip_and_refuse_by_field() {
        let file = vectors::load("send-status.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<SendStatusRequest>(
                case["name"].as_str().unwrap(),
                &case["request"],
            );
        }
        for case in vectors::cases(&file, "refused_requests") {
            let name = case["name"].as_str().unwrap();
            let field = case["field"].as_str().unwrap();
            let error = serde_json::from_value::<SendStatusRequest>(case["request"].clone())
                .expect_err(name);
            assert!(error.to_string().contains(field), "{name}: {error}");
        }
        assert_eq!(
            serde_json::to_value(SendStatusRequest::new("s-1")).unwrap(),
            serde_json::json!({"send_id": "s-1"})
        );
    }

    /// Each pinned answer is a `SendReply`. A delivered queued send names
    /// the run that carried it beside its receipt; a pending one names its
    /// submission and no run.
    #[test]
    fn send_status_replies_are_send_replies() {
        let file = vectors::load("send-status.json");
        let mut seen = std::collections::BTreeSet::new();
        for case in vectors::cases(&file, "replies") {
            let name = case["name"].as_str().unwrap();
            let reply: SendStatusReply = vectors::round_trip(name, &case["reply"]);
            match reply.delivered.as_ref().map(|d| d.r#as.as_str()) {
                Some(delivered_as::STEP) => {
                    let delivered = reply.delivered.as_ref().unwrap();
                    assert!(delivered.r#ref.is_some(), "{name}: a step names its row");
                    assert!(reply.run_id.is_some(), "{name}: a step names its run");
                }
                Some(delivered_as::TURN) => {
                    let delivered = reply.delivered.as_ref().unwrap();
                    assert!(reply.run_id.is_some(), "{name}");
                    assert_eq!(
                        delivered.r#ref, reply.run_id,
                        "{name}: a turn's ref is its run"
                    );
                }
                Some(delivered_as::UNKNOWN) => {}
                None => {
                    assert_eq!(reply.send_state(), SendState::Pending, "{name}");
                    assert!(reply.submission_id.is_some(), "{name}");
                    assert!(reply.run_id.is_none(), "{name}");
                }
                Some(other) => panic!("{name}: unexpected receipt {other}"),
            }
            seen.insert(
                reply
                    .delivered
                    .as_ref()
                    .map_or(states::PENDING.to_owned(), |d| d.r#as.clone()),
            );
        }
        for wanted in [
            states::PENDING,
            delivered_as::STEP,
            delivered_as::TURN,
            delivered_as::UNKNOWN,
        ] {
            assert!(seen.contains(wanted), "no send.status vector for {wanted}");
        }
    }

    #[test]
    fn an_unrecorded_send_id_is_refused_unknown_send() {
        let file = vectors::load("send-status.json");
        let refusals = vectors::cases(&file, "refusals");
        assert!(!refusals.is_empty());
        for case in refusals {
            let name = case["name"].as_str().unwrap();
            vectors::round_trip::<SendStatusRequest>(name, &case["request"]);
            let code = case["refusal"]["code"].as_str().unwrap();
            assert!(errors::CODES.contains(&code), "{name}");
            assert_eq!(
                errors::is_retryable(code),
                case["retryable"].as_bool().unwrap(),
                "{name}"
            );
            assert_eq!(code, errors::UNKNOWN_SEND, "{name}");
            assert!(case["refusal"].get("detail").is_none(), "{name}");
        }
    }
}
