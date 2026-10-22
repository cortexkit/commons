//! The item image shape every model-batch role shares, its validation, and
//! its canonical form.
//!
//! An image is a `data:` URL string or `{content_type, base64}`. Remote URLs
//! are never accepted. Validation decodes the base64, so an image a module
//! admits is one the provider can read, and sizes are the decoded bytes.
//! The canonical form, `{content_type, base64}` with the decoded bytes
//! re-encoded as standard padded base64, is what batch identity hashes: one
//! image sent in either spelling is the same batch (`RULES.md` §2, §13).

use base64::{
    alphabet,
    engine::{general_purpose::STANDARD, DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig},
    Engine,
};
use serde::{Deserialize, Serialize};

use crate::errors::Refusal;

/// Image content types a provider accepts.
pub mod content_types {
    pub const PNG: &str = "image/png";
    pub const JPEG: &str = "image/jpeg";
    pub const WEBP: &str = "image/webp";
    pub const ALL: &[&str] = &[PNG, JPEG, WEBP];
}

/// Largest image, in decoded bytes (4 MiB), as Cloudflare Workers AI states
/// it for its image-taking batch models.
pub const IMAGE_MAX_BYTES: usize = 4 * 1024 * 1024;
/// Largest total of an item's images, in decoded bytes (8 MiB).
pub const IMAGES_MAX_TOTAL_BYTES: usize = 8 * 1024 * 1024;
/// Largest image, in pixels (16 megapixels). The role crates do not decode
/// image formats, so this is a runner's check, not [`check_image`]'s.
pub const IMAGE_MAX_PIXELS: u64 = 16_000_000;

/// Standard base64, padded or not. Trailing bits that a canonical encoding
/// would leave zero are refused, so one byte string has one accepted
/// spelling per padding choice.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// An image in its object form: `{content_type, base64}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InlineImage {
    /// One of the model's accepted formats ([`content_types::ALL`] unless
    /// the model states fewer).
    pub content_type: String,
    pub base64: String,
}

impl InlineImage {
    pub fn new(content_type: impl Into<String>, base64: impl Into<String>) -> Self {
        Self {
            content_type: content_type.into(),
            base64: base64.into(),
        }
    }
}

/// An image: a `data:` URL string, or `{content_type, base64}`. Remote URLs
/// are never accepted.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Image {
    DataUrl(String),
    Inline(InlineImage),
}

/// An image's content type and decoded bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl Image {
    /// The image's size in decoded bytes, estimated from its base64 length.
    /// Exact for well-formed base64; [`Image::decode`] is the check.
    pub fn approx_bytes(&self) -> usize {
        let payload = match self {
            Self::DataUrl(url) => url.split_once(',').map_or("", |(_, data)| data),
            Self::Inline(inline) => &inline.base64,
        };
        payload.trim_end_matches('=').len() * 3 / 4
    }

    /// The content type and decoded bytes, or why the image is malformed.
    /// A `data:` URL must be `data:<content type>;base64,<payload>`. The
    /// content type is not checked against a format list here.
    pub fn decode(&self) -> Result<DecodedImage, String> {
        let (content_type, payload) = match self {
            Self::DataUrl(url) => {
                let Some(rest) = url.strip_prefix("data:") else {
                    return Err(
                        "an image string must be a data: URL; remote URLs are refused".into(),
                    );
                };
                let Some((media, payload)) = rest.split_once(',') else {
                    return Err("a data: URL has no payload".into());
                };
                let Some(content_type) = media.strip_suffix(";base64") else {
                    return Err("a data: URL is data:<content type>;base64,<payload>".into());
                };
                (content_type, payload)
            }
            Self::Inline(inline) => (inline.content_type.as_str(), inline.base64.as_str()),
        };
        let bytes = LENIENT
            .decode(payload)
            .map_err(|e| format!("the image's base64 does not decode: {e}"))?;
        if bytes.is_empty() {
            return Err("an image has no bytes".into());
        }
        Ok(DecodedImage {
            content_type: content_type.to_owned(),
            bytes,
        })
    }

    /// The canonical form batch identity hashes: the content type and the
    /// decoded bytes as standard padded base64.
    pub fn canonical(&self) -> Result<InlineImage, String> {
        let decoded = self.decode()?;
        Ok(InlineImage::new(
            decoded.content_type,
            STANDARD.encode(decoded.bytes),
        ))
    }
}

/// A model's image limits per item, as an extractor `role.describe` entry
/// states them: `{max, max_bytes_each, max_bytes_total, formats}`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ImageLimits {
    /// Most images in one item.
    pub max: u64,
    /// Largest image, in decoded bytes.
    pub max_bytes_each: u64,
    /// Largest total of one item's images, in decoded bytes.
    pub max_bytes_total: u64,
    /// The content types accepted, a subset of [`content_types::ALL`].
    pub formats: Vec<String>,
}

impl ImageLimits {
    pub fn new(max: u64, max_bytes_each: u64, max_bytes_total: u64, formats: Vec<String>) -> Self {
        Self {
            max,
            max_bytes_each,
            max_bytes_total,
            formats,
        }
    }

    /// Workers AI's limits ([`IMAGE_MAX_BYTES`] each,
    /// [`IMAGES_MAX_TOTAL_BYTES`] in total, every format) with at most
    /// `max` images.
    pub fn workers_ai(max: u64) -> Self {
        Self::new(
            max,
            IMAGE_MAX_BYTES as u64,
            IMAGES_MAX_TOTAL_BYTES as u64,
            content_types::ALL.iter().map(|t| (*t).to_owned()).collect(),
        )
    }
}

/// Check one image's form, content type, base64 and decoded size against
/// `limits`, returning its decoded size. The error says why it is refused.
pub fn check_image_with(image: &Image, limits: &ImageLimits) -> Result<u64, String> {
    if let Image::DataUrl(url) = image {
        if !url.starts_with("data:") {
            return Err("an image string must be a data: URL; remote URLs are refused".into());
        }
    }
    let content_type = match image {
        Image::DataUrl(url) => url
            .split_once(',')
            .map(|(media, _)| media["data:".len()..].split(';').next().unwrap_or(""))
            .ok_or_else(|| "a data: URL has no payload".to_owned())?,
        Image::Inline(inline) => inline.content_type.as_str(),
    };
    if !limits.formats.iter().any(|format| format == content_type) {
        return Err(format!("unsupported image content type {content_type}"));
    }
    let size = image.decode()?.bytes.len() as u64;
    if size > limits.max_bytes_each {
        return Err(format!(
            "an image is at most {} bytes",
            limits.max_bytes_each
        ));
    }
    Ok(size)
}

/// Check one image against Workers AI's limits ([`ImageLimits::workers_ai`]).
pub fn check_image(image: &Image) -> Result<(), String> {
    check_image_with(image, &ImageLimits::workers_ai(u64::MAX)).map(|_| ())
}

/// Check an item's images: their count against `limits.max` (`None` when
/// the model takes no images), each image ([`check_image_with`]), and their
/// decoded total. `field` is the item's `images` path (`items[i].images`);
/// a refusal names it, or `field[j]` for one image.
pub fn check_images(
    model: &str,
    field: &str,
    images: &[Image],
    limits: Option<&ImageLimits>,
) -> Result<(), Refusal> {
    let max = limits.map_or(0, |limits| limits.max);
    if images.len() as u64 > max {
        return Err(Refusal::over_limit(
            field,
            max,
            if limits.is_none() {
                format!("{model} takes no images")
            } else {
                format!("{} images; at most {max}", images.len())
            },
        ));
    }
    let Some(limits) = limits else {
        return Ok(());
    };
    let mut total: u64 = 0;
    for (j, image) in images.iter().enumerate() {
        let size = check_image_with(image, limits)
            .map_err(|why| Refusal::invalid_params(format!("{field}[{j}]"), why))?;
        total = total.saturating_add(size);
    }
    if total > limits.max_bytes_total {
        return Err(Refusal::over_limit(
            field,
            limits.max_bytes_total,
            format!("{total} image bytes; at most {}", limits.max_bytes_total),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_urls_unknown_types_and_bad_base64_are_refused() {
        assert!(check_image(&Image::DataUrl("https://example.com/a.png".into())).is_err());
        assert!(check_image(&Image::DataUrl("data:image/gif;base64,AAAA".into())).is_err());
        assert!(check_image(&Image::DataUrl("data:image/png;base64,AAAA".into())).is_ok());
        assert!(check_image(&Image::Inline(InlineImage::new("image/webp", "AAAA"))).is_ok());
        assert!(check_image(&Image::Inline(InlineImage::new("image/bmp", "AAAA"))).is_err());
        let malformed =
            check_image(&Image::Inline(InlineImage::new("image/png", "A!A="))).unwrap_err();
        assert!(malformed.contains("does not decode"), "{malformed}");
        let plain = check_image(&Image::DataUrl("data:image/png,AAAA".into())).unwrap_err();
        assert!(plain.contains(";base64"), "{plain}");
        assert!(check_image(&Image::Inline(InlineImage::new("image/png", ""))).is_err());
    }

    #[test]
    fn both_spellings_have_one_canonical_form() {
        let url = Image::DataUrl("data:image/png;base64,AAECAw".into());
        let inline = Image::Inline(InlineImage::new("image/png", "AAECAw=="));
        assert_eq!(url.canonical().unwrap(), inline.canonical().unwrap());
        assert_eq!(url.canonical().unwrap().base64, "AAECAw==");
    }

    #[test]
    fn decoded_sizes_are_checked_against_the_limits() {
        let limits = ImageLimits::new(2, 3, 5, vec![content_types::PNG.into()]);
        let three = Image::Inline(InlineImage::new("image/png", "AAAA"));
        let four = Image::Inline(InlineImage::new("image/png", "AAAAAA=="));
        assert_eq!(check_image_with(&three, &limits), Ok(3));
        assert!(check_image_with(&four, &limits).is_err());
        let refusal = check_images(
            "m",
            "items[0].images",
            &[three.clone(), three.clone()],
            Some(&limits),
        )
        .unwrap_err();
        assert_eq!(refusal.field(), Some("items[0].images"));
        assert_eq!(refusal.detail.limit, Some(5));
        let refusal = check_images("m", "items[0].images", &[three], None).unwrap_err();
        assert_eq!(refusal.detail.limit, Some(0));
        assert_eq!(refusal.message, "m takes no images");
    }
}
