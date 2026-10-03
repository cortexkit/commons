//! The batch body identity: what decides whether a re-sent `batch_id` is a
//! replay or `batch_id_reuse`.
//!
//! The identity is the model, the questions, and each item, the questions
//! and every item hashed as RFC 8785 (JCS) canonical JSON. Canonicalizing
//! first means the key order of an object `state`, whitespace, and number
//! spellings JSON treats as equal (`1.0` and `1`, `1e30` and `1E+30`) can't
//! turn a retry into a reuse. `max_cost_usd` is not part of it: a re-send
//! may carry another ceiling (see
//! [`effective_ceiling`](crate::request::effective_ceiling)).

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::request::ClassifyRequest;

/// A batch's body identity, as a runner records it at first admission.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct BatchIdentity {
    /// The catalog id, compared as is.
    pub model: String,
    /// SHA-256 of the JCS of `questions`, lowercase hex.
    pub questions: String,
    /// SHA-256 of the JCS of each item, lowercase hex, in request order.
    pub items: Vec<String>,
}

/// Why an identity could not be computed: the request holds a value JCS
/// cannot encode.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityError(pub String);

impl std::fmt::Display for IdentityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "request cannot be canonicalized: {}", self.0)
    }
}

impl std::error::Error for IdentityError {}

/// The RFC 8785 (JCS) canonical JSON bytes of `value`.
pub fn canonical_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, IdentityError> {
    serde_jcs::to_vec(value).map_err(|e| IdentityError(e.to_string()))
}

/// SHA-256 of the canonical JSON of `value`, as 64 lowercase hex characters.
pub fn digest<T: Serialize + ?Sized>(value: &T) -> Result<String, IdentityError> {
    let digest = Sha256::digest(canonical_json(value)?);
    Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

impl BatchIdentity {
    /// The identity of `request`.
    pub fn of(request: &ClassifyRequest) -> Result<Self, IdentityError> {
        Ok(Self {
            model: request.model.clone(),
            questions: digest(&request.questions)?,
            items: request.items.iter().map(digest).collect::<Result<_, _>>()?,
        })
    }

    /// The first field in which `sent` differs from this recorded identity,
    /// as `batch_id_reuse` names it: `model`, `questions`, `items` (a
    /// different item count) or `items[i]`. `None` when they are the same
    /// body, which makes the re-send a replay.
    pub fn first_difference(&self, sent: &Self) -> Option<String> {
        if self.model != sent.model {
            return Some("model".into());
        }
        if self.questions != sent.questions {
            return Some("questions".into());
        }
        if self.items.len() != sent.items.len() {
            return Some("items".into());
        }
        self.items
            .iter()
            .zip(&sent.items)
            .position(|(recorded, sent)| recorded != sent)
            .map(|i| format!("items[{i}]"))
    }
}

#[cfg(test)]
mod tests {
    use serde_json::{json, Number};

    use super::*;
    use crate::{
        question::Question,
        request::{parse_request, Item},
    };

    fn request(states: Vec<serde_json::Value>) -> ClassifyRequest {
        ClassifyRequest::new(
            "b",
            "cloudflare/clef",
            [("q".to_owned(), Question::noul("i"))].into(),
            states.into_iter().map(Item::new).collect(),
        )
    }

    #[test]
    fn key_order_and_number_spelling_do_not_change_the_identity() {
        let first = parse_request(
            r#"{"batch_id":"b","model":"m","questions":{"q":{"type":"noul","instructions":"i"}},"items":[{"state":{"b":1.0,"a":1e30,"c":{"y":[2.50],"x":0}}}]}"#,
        )
        .unwrap();
        let second = parse_request(
            r#"{"batch_id":"b","model":"m","questions":{"q":{"type":"noul","instructions":"i"}},"items":[{"state":{"c":{"x":0,"y":[2.5]},"a":1E+30,"b":1}}]}"#,
        )
        .unwrap();
        assert_eq!(
            String::from_utf8(canonical_json(&first.items[0].state).unwrap()).unwrap(),
            r#"{"a":1e+30,"b":1,"c":{"x":0,"y":[2.5]}}"#
        );
        assert_eq!(
            BatchIdentity::of(&first).unwrap(),
            BatchIdentity::of(&second).unwrap()
        );
    }

    #[test]
    fn plain_json_would_have_called_the_same_state_different() {
        // The reason for canonicalizing: serde_json writes the float `1.0`
        // and the integer `1` differently, so hashing its text would refuse
        // a faithful retry as reuse.
        let float: serde_json::Value = serde_json::from_str(r#"{"a":1.0}"#).unwrap();
        let int: serde_json::Value = serde_json::from_str(r#"{"a":1}"#).unwrap();
        assert_ne!(
            serde_json::to_string(&float).unwrap(),
            serde_json::to_string(&int).unwrap()
        );
        assert_eq!(digest(&float).unwrap(), digest(&int).unwrap());
    }

    #[test]
    fn the_first_difference_is_named() {
        let recorded = BatchIdentity::of(&request(vec![json!("a"), json!("b")])).unwrap();
        let same = BatchIdentity::of(&request(vec![json!("a"), json!("b")])).unwrap();
        assert_eq!(recorded.first_difference(&same), None);
        let changed = BatchIdentity::of(&request(vec![json!("a"), json!("c")])).unwrap();
        assert_eq!(recorded.first_difference(&changed), Some("items[1]".into()));
        let longer = BatchIdentity::of(&request(vec![json!("a"), json!("b"), json!("c")])).unwrap();
        assert_eq!(recorded.first_difference(&longer), Some("items".into()));
        let mut other_questions = request(vec![json!("a"), json!("b")]);
        other_questions
            .questions
            .insert("r".into(), Question::noul("j"));
        assert_eq!(
            recorded.first_difference(&BatchIdentity::of(&other_questions).unwrap()),
            Some("questions".into())
        );
        let mut other_model = request(vec![json!("a"), json!("b")]);
        other_model.model = "typesafe/jev-latest".into();
        assert_eq!(
            recorded.first_difference(&BatchIdentity::of(&other_model).unwrap()),
            Some("model".into())
        );
    }

    #[test]
    fn max_cost_usd_is_not_part_of_the_identity() {
        let plain = request(vec![json!("a")]);
        let capped = request(vec![json!("a")]).with_max_cost_usd(Number::from_f64(0.01).unwrap());
        assert_eq!(
            BatchIdentity::of(&plain).unwrap(),
            BatchIdentity::of(&capped).unwrap()
        );
    }
}
