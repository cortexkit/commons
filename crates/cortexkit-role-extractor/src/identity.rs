//! The batch body identity: what decides whether a re-sent `batch_id` is a
//! replay or `batch_id_reuse`.
//!
//! The identity is the model, the fields, the options in force, and each
//! item, hashed as the shared model-batch rules say (`cortexkit-model-batch`,
//! its `RULES.md` §2): RFC 8785 (JCS) canonical JSON, with an item's images
//! in canonical form. The options are hashed as they are in force
//! ([`EffectiveOptions`](crate::request::EffectiveOptions)), so an absent
//! option and the same option sent as `false` are one batch; they are part
//! of the identity because they change what the provider answers.
//! `max_cost_usd` is not part of it.

use serde::{Deserialize, Serialize};

pub use cortexkit_model_batch::identity::{canonical_json, digest, IdentityError};
use cortexkit_model_batch::identity::{first_difference, item_digest};

use crate::request::ExtractRequest;

/// A batch's body identity, as a runner records it at first admission.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct BatchIdentity {
    /// The catalog id, compared as is.
    pub model: String,
    /// SHA-256 of the JCS of `fields`, lowercase hex.
    pub fields: String,
    /// SHA-256 of the JCS of the options in force, lowercase hex.
    pub options: String,
    /// SHA-256 of the JCS of each item, its images in canonical form,
    /// lowercase hex, in request order.
    pub items: Vec<String>,
}

impl BatchIdentity {
    /// The identity of `request`. Fails only for an image that does not
    /// decode, which validation refuses first.
    pub fn of(request: &ExtractRequest) -> Result<Self, IdentityError> {
        Ok(Self {
            model: request.model.clone(),
            fields: digest(&request.fields)?,
            options: digest(&request.effective_options())?,
            items: request
                .items
                .iter()
                .map(|item| item_digest(item, item.images.as_deref()))
                .collect::<Result<_, _>>()?,
        })
    }

    /// The first part in which `sent` differs from this recorded identity,
    /// as `batch_id_reuse` names it: `model`, `fields`, `options`, `items`
    /// (a different item count) or `items[i]`, checked in that order.
    /// `None` when they are the same body, which makes the re-send a replay.
    pub fn first_difference(&self, sent: &Self) -> Option<String> {
        first_difference(
            &[
                ("model", &self.model, &sent.model),
                ("fields", &self.fields, &sent.fields),
                ("options", &self.options, &sent.options),
            ],
            &self.items,
            &sent.items,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        field::{Field, FieldType},
        image::{Image, InlineImage},
        request::{Item, Options},
    };

    fn request(items: Vec<Item>) -> ExtractRequest {
        ExtractRequest::new(
            "b",
            "cloudflare-workers-ai/@cf/cloudflare/clairic",
            [("total".to_owned(), Field::new(FieldType::Number))].into(),
            items,
        )
    }

    fn id(request: &ExtractRequest) -> BatchIdentity {
        BatchIdentity::of(request).unwrap()
    }

    #[test]
    fn parts_are_named_in_order() {
        let recorded = id(&request(vec![Item::new("a"), Item::new("b")]));
        assert_eq!(
            recorded.first_difference(&id(&request(vec![Item::new("a"), Item::new("b")]))),
            None
        );
        assert_eq!(
            recorded.first_difference(&id(&request(vec![Item::new("a"), Item::new("c")]))),
            Some("items[1]".into())
        );
        assert_eq!(
            recorded.first_difference(&id(&request(vec![Item::new("a")]))),
            Some("items".into())
        );
        let quantiles =
            request(vec![Item::new("a"), Item::new("b")]).with_options(Options::new(true, false));
        assert_eq!(
            recorded.first_difference(&id(&quantiles)),
            Some("options".into())
        );
        let mut fields = request(vec![Item::new("a"), Item::new("b")]);
        fields
            .fields
            .insert("due".into(), Field::new(FieldType::Date));
        assert_eq!(
            recorded.first_difference(&id(&fields)),
            Some("fields".into())
        );
        let dated = request(vec![
            Item::new("a").with_reference_date("2026-01-01"),
            Item::new("b"),
        ]);
        assert_eq!(
            recorded.first_difference(&id(&dated)),
            Some("items[0]".into())
        );
    }

    #[test]
    fn absent_options_are_their_defaults_and_images_hash_canonically() {
        let plain = id(&request(vec![Item::new("a")]));
        let stated = id(&request(vec![Item::new("a")]).with_options(Options::new(false, false)));
        assert_eq!(plain.first_difference(&stated), None);
        let url =
            Item::new("").with_images(vec![Image::DataUrl("data:image/png;base64,AAECAw".into())]);
        let inline = Item::new("").with_images(vec![Image::Inline(InlineImage::new(
            "image/png",
            "AAECAw==",
        ))]);
        assert_eq!(
            id(&request(vec![url])).first_difference(&id(&request(vec![inline]))),
            None
        );
    }
}
