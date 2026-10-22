//! The batch body identity: what decides whether a re-sent `batch_id` is a
//! replay or `batch_id_reuse`.
//!
//! The identity is the model, the questions, and each item, the questions
//! and every item hashed as RFC 8785 (JCS) canonical JSON, with an item's
//! images in their canonical form. These are the shared model-batch rules
//! (`cortexkit-model-batch`, its `RULES.md` §2); this module names the
//! parts `classifier/v1` hashes. Canonicalizing first means the key order
//! of an object `state`, whitespace, number spellings JSON treats as equal
//! (`1.0` and `1`, `1e30` and `1E+30`), and the spelling of an image (a
//! `data:` URL or `{content_type, base64}`) can't turn a retry into a
//! reuse. `max_cost_usd` is not part of it: a re-send may carry another
//! ceiling (see [`effective_ceiling`](crate::request::effective_ceiling)).

use serde::{Deserialize, Serialize};

pub use cortexkit_model_batch::identity::{canonical_json, digest, IdentityError};
use cortexkit_model_batch::identity::{first_difference, item_digest};

use crate::request::ClassifyRequest;

/// A batch's body identity, as a runner records it at first admission.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct BatchIdentity {
    /// The catalog id, compared as is.
    pub model: String,
    /// SHA-256 of the JCS of `questions`, lowercase hex.
    pub questions: String,
    /// SHA-256 of the JCS of each item, its images in canonical form,
    /// lowercase hex, in request order.
    pub items: Vec<String>,
}

impl BatchIdentity {
    /// The identity of `request`. Fails only for an image that does not
    /// decode, which validation refuses first.
    pub fn of(request: &ClassifyRequest) -> Result<Self, IdentityError> {
        Ok(Self {
            model: request.model.clone(),
            questions: digest(&request.questions)?,
            items: request
                .items
                .iter()
                .map(|item| item_digest(item, item.images.as_deref()))
                .collect::<Result<_, _>>()?,
        })
    }

    /// The first field in which `sent` differs from this recorded identity,
    /// as `batch_id_reuse` names it: `model`, `questions`, `items` (a
    /// different item count) or `items[i]`. `None` when they are the same
    /// body, which makes the re-send a replay.
    pub fn first_difference(&self, sent: &Self) -> Option<String> {
        first_difference(
            &[
                ("model", &self.model, &sent.model),
                ("questions", &self.questions, &sent.questions),
            ],
            &self.items,
            &sent.items,
        )
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
            "cloudflare-workers-ai/@cf/cloudflare/clef",
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
        other_model.model = "typesafe/jev-1.13.0".into();
        assert_eq!(
            recorded.first_difference(&BatchIdentity::of(&other_model).unwrap()),
            Some("model".into())
        );
    }

    #[test]
    fn one_image_in_either_spelling_is_one_identity() {
        use crate::question::{Image, InlineImage};
        let with = |image: Image| {
            let mut request = request(vec![json!("s")]);
            request.items[0] = Item::new("s").with_images(vec![image]);
            BatchIdentity::of(&request).unwrap()
        };
        let url = with(Image::DataUrl("data:image/png;base64,AAECAw".into()));
        let inline = with(Image::Inline(InlineImage::new("image/png", "AAECAw==")));
        assert_eq!(url.first_difference(&inline), None);
        let other = with(Image::Inline(InlineImage::new("image/png", "AAECBA==")));
        assert_eq!(url.first_difference(&other), Some("items[0]".into()));
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
