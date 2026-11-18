//! `agent_data.release`: on the source, once the move record reads
//! `activated` (the receiver has made the data live).
//! The write freeze that `prepare` set becomes a durable
//! tombstone naming the new holder, and the old copy is hidden. The reply
//! says whether later local writes are blocked ([`ReleaseReply::enforced`]):
//! when they are, the participant refuses its own local callers' writes to
//! these refs. The participant's own refusal can name the new holder by
//! `holderBox` when supplied, never by the `holder` pseudonym. When writes
//! are not blocked, anything written afterwards becomes a fork that
//! `diverged` reports on a move back. The refs must be the set `prepare`
//! named for this `cutId`, in any order; another set is refused
//! `invalid_state` ([`crate::step::check_ref_set`]).
//! Step: `exported` → `released`.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

use serde::{de::Error as _, Deserialize, Deserializer, Serialize};

use crate::refs::{is_lower_hex, CutId, Refs};

/// The `release` request: `{refs, cutId, holder, holderBox?}`. Unknown
/// members and malformed values are refused `invalid_request`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReleaseRequest {
    pub refs: Refs,
    #[serde(rename = "cutId")]
    pub cut_id: CutId,
    /// The new holder's author pseudonym.
    pub holder: String,
    /// The new holder's box id, not its pseudonym. Omit when unknown.
    #[serde(
        rename = "holderBox",
        default,
        deserialize_with = "deserialize_holder_box",
        skip_serializing_if = "Option::is_none"
    )]
    pub holder_box: Option<String>,
}

impl ReleaseRequest {
    pub fn new(refs: Refs, cut_id: CutId, holder: impl Into<String>) -> Self {
        Self {
            refs,
            cut_id,
            holder: holder.into(),
            holder_box: None,
        }
    }

    /// Set the box id, which must be exactly 32 lowercase hex characters.
    pub fn with_holder_box(mut self, holder_box: impl Into<String>) -> Result<Self, &'static str> {
        self.holder_box = Some(check_holder_box(holder_box.into())?);
        Ok(self)
    }
}

fn check_holder_box(holder_box: String) -> Result<String, &'static str> {
    if is_lower_hex(&holder_box, 32) {
        Ok(holder_box)
    } else {
        Err("holderBox must be exactly 32 lowercase hex characters")
    }
}

fn deserialize_holder_box<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    let holder_box = String::deserialize(deserializer)
        .map_err(|error| D::Error::custom(format_args!("holderBox: {error}")))?;
    check_holder_box(holder_box)
        .map(Some)
        .map_err(D::Error::custom)
}

/// The `release` reply: `{enforced}`. Strict, and `enforced` is required:
/// a reply without it does not decode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ReleaseReply {
    /// `true`: later local writes for these refs are refused, naming the
    /// holder by box id when supplied. `false`: the tombstone is recorded and
    /// the old copy hidden, but writes are not blocked, so anything written
    /// afterwards becomes a fork that `diverged` reports on a move back.
    pub enforced: bool,
}

impl ReleaseReply {
    pub fn new(enforced: bool) -> Self {
        Self { enforced }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        errors::RefusalCode,
        idempotency::{compare, Repeat},
        request::Request,
        Op,
    };
    use serde_json::json;

    const BOX: &str = "0123456789abcdef0123456789abcdef";
    const CUT: &str = "7f3a9c2e5b8d41f6a0c3e9b7d2f84a15";

    #[test]
    fn release_holder_box_participates_in_repeats() {
        let request =
            ReleaseRequest::new(Refs::new(vec![]).unwrap(), CutId::new(CUT).unwrap(), "p");
        let without_box = Request::Release(request.clone());
        let with_box = Request::Release(request.clone().with_holder_box(BOX).unwrap());
        let another_box = Request::Release(
            request
                .with_holder_box("fedcba9876543210fedcba9876543210")
                .unwrap(),
        );

        let decoded = Request::decode(Op::Release.method(), &with_box.params()).unwrap();
        assert_eq!(compare(&with_box, &decoded), Repeat::Replay);
        for (first, repeat) in [
            (&with_box, &another_box),
            (&with_box, &without_box),
            (&without_box, &with_box),
        ] {
            assert_eq!(
                compare(first, repeat).refusal(),
                Some(RefusalCode::IdReused)
            );
        }
    }

    #[test]
    fn release_refuses_malformed_holder_box_and_names_the_member() {
        for malformed in [
            json!("0123456789ABCDEF0123456789ABCDEF"),
            json!(&BOX[..31]),
            json!(format!("{BOX}0")),
            json!("0123456789abcdef0123456789abcdeg"),
            json!(null),
            json!(42),
        ] {
            let params = json!({"refs": [], "cutId": CUT, "holder": "p", "holderBox": malformed});
            let error = Request::decode(Op::Release.method(), &params).unwrap_err();
            assert_eq!(error.code, RefusalCode::InvalidRequest, "{malformed}");
            assert!(
                error.message.contains("holderBox"),
                "{malformed}: {error:?}"
            );
        }
    }

    #[test]
    fn release_reply_requires_enforced() {
        for enforced in [true, false] {
            let wire = serde_json::json!({ "enforced": enforced });
            let reply: ReleaseReply = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(reply, ReleaseReply::new(enforced));
            assert_eq!(serde_json::to_value(reply).unwrap(), wire);
        }
        assert!(serde_json::from_value::<ReleaseReply>(serde_json::json!({})).is_err());
        assert!(
            serde_json::from_value::<ReleaseReply>(serde_json::json!({"enforced": "yes"})).is_err()
        );
    }
}
