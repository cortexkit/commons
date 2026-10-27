//! Batch identity: what decides whether a re-sent `batch_id` is a replay or
//! `batch_id_reuse` (`RULES.md` §2).
//!
//! Each part of a batch body is hashed as RFC 8785 (JCS) canonical JSON, so
//! the key order of an object, whitespace, and number spellings JSON treats
//! as equal (`1.0` and `1`, `1e30` and `1E+30`) can't turn a retry into a
//! reuse. An item's images are hashed in their canonical form
//! ([`Image::canonical`]), so a `data:` URL and the same image as
//! `{content_type, base64}` are the same item. A reference is hashed as
//! `{handle}`, never the bytes it resolves to: checking a re-sent
//! `batch_id` never needs a handle that may have expired, and a reference
//! and an inline copy of the same picture are different items. Each role
//! names its parts and their order; [`first_difference`] names the first
//! that differs.

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::image::Image;

/// Why an identity could not be computed: the request holds a value JCS
/// cannot encode, or an image that does not decode.
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

/// The digest of one item: the item as JSON, with its `images` member, when
/// present, replaced by each image's canonical form. `images` is the item's
/// image list as the role decoded it.
pub fn item_digest<T: Serialize + ?Sized>(
    item: &T,
    images: Option<&[Image]>,
) -> Result<String, IdentityError> {
    let mut value = serde_json::to_value(item).map_err(|e| IdentityError(e.to_string()))?;
    if let (Some(images), Value::Object(members)) = (images, &mut value) {
        let canonical = images
            .iter()
            .enumerate()
            .map(|(j, image)| {
                image
                    .canonical()
                    .map_err(|why| IdentityError(format!("images[{j}]: {why}")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        members.insert(
            "images".into(),
            serde_json::to_value(canonical).map_err(|e| IdentityError(e.to_string()))?,
        );
    }
    digest(&value)
}

/// The first part in which `sent` differs from `recorded`, as
/// `batch_id_reuse` names it. `parts` are the role's named parts in the
/// order it checks them, each `(name, recorded, sent)`; then the item count
/// (`items`) and the first differing item (`items[i]`). `None` when every
/// part and item is the same, which makes the re-send a replay.
pub fn first_difference(
    parts: &[(&str, &str, &str)],
    recorded_items: &[String],
    sent_items: &[String],
) -> Option<String> {
    if let Some((name, _, _)) = parts.iter().find(|(_, recorded, sent)| recorded != sent) {
        return Some((*name).to_owned());
    }
    if recorded_items.len() != sent_items.len() {
        return Some("items".into());
    }
    recorded_items
        .iter()
        .zip(sent_items)
        .position(|(recorded, sent)| recorded != sent)
        .map(|i| format!("items[{i}]"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::image::{ImageReference, InlineImage};

    #[test]
    fn number_spelling_and_key_order_do_not_change_a_digest() {
        let float: Value = serde_json::from_str(r#"{"a":1.0,"b":{"y":[2.50],"x":0}}"#).unwrap();
        let int: Value = serde_json::from_str(r#"{"b":{"x":0,"y":[2.5]},"a":1}"#).unwrap();
        assert_ne!(
            serde_json::to_string(&float).unwrap(),
            serde_json::to_string(&int).unwrap()
        );
        assert_eq!(digest(&float).unwrap(), digest(&int).unwrap());
    }

    #[test]
    fn an_image_in_either_spelling_is_the_same_item() {
        let url = vec![Image::DataUrl("data:image/png;base64,AAECAw".into())];
        let inline = vec![Image::Inline(InlineImage::new("image/png", "AAECAw=="))];
        let a = json!({"state": "s", "images": url});
        let b = json!({"state": "s", "images": inline});
        assert_ne!(digest(&a).unwrap(), digest(&b).unwrap());
        assert_eq!(
            item_digest(&a, Some(&url)).unwrap(),
            item_digest(&b, Some(&inline)).unwrap()
        );
        let other = vec![Image::Inline(InlineImage::new("image/jpeg", "AAECAw=="))];
        let c = json!({"state": "s", "images": other});
        assert_ne!(
            item_digest(&a, Some(&url)).unwrap(),
            item_digest(&c, Some(&other)).unwrap()
        );
    }

    #[test]
    fn a_reference_is_hashed_as_its_handle_never_its_bytes() {
        let item = |images: &Vec<Image>| {
            item_digest(&json!({"state": "s", "images": images}), Some(images)).unwrap()
        };
        let shot = vec![Image::Reference(ImageReference::new("shot:1"))];
        let again = vec![Image::Reference(ImageReference::new("shot:1"))];
        let other = vec![Image::Reference(ImageReference::new("shot:2"))];
        let inline = vec![Image::Inline(InlineImage::new("image/png", "AAECAw=="))];
        assert_eq!(item(&shot), item(&again));
        assert_ne!(item(&shot), item(&other));
        assert_ne!(item(&shot), item(&inline));
        assert_eq!(
            item(&shot),
            digest(&json!({"state": "s", "images": [{"handle": "shot:1"}]})).unwrap()
        );
    }

    #[test]
    fn the_first_differing_part_is_named_in_order() {
        let items = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let parts = [
            ("model", "m", "m"),
            ("fields", "f", "g"),
            ("options", "o", "p"),
        ];
        assert_eq!(
            first_difference(&parts, &items(&["a"]), &items(&["b"])),
            Some("fields".into())
        );
        let same = [("model", "m", "m")];
        assert_eq!(
            first_difference(&same, &items(&["a", "b"]), &items(&["a", "c"])),
            Some("items[1]".into())
        );
        assert_eq!(
            first_difference(&same, &items(&["a"]), &items(&["a", "b"])),
            Some("items".into())
        );
        assert_eq!(
            first_difference(&same, &items(&["a"]), &items(&["a"])),
            None
        );
    }
}
