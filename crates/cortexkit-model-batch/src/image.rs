//! The item image shape every model-batch role shares, its validation, and
//! its canonical form.
//!
//! An image is a `data:` URL string, `{content_type, base64}`, or a
//! reference `{handle}` to an image another module already holds. Remote
//! URLs are never accepted. Validation decodes the base64, so an image a
//! module admits is one the provider can read, and sizes are the decoded
//! bytes. The canonical form is what batch identity hashes (`RULES.md` §2,
//! §13): `{content_type, base64}` with the decoded bytes re-encoded as
//! standard padded base64 for an image that carries its bytes, so one image
//! sent in either spelling is the same batch; and `{handle}` for a
//! reference, never the bytes it resolves to.
//!
//! A reference carries no bytes until the host resolves it, through the
//! module that issued the handle, just before it calls the model. So the
//! byte-reading methods ([`Image::decode`], [`Image::approx_bytes`]) return
//! `None` for a reference, and the checks come in two forms:
//!
//! - [`check_images`] for a host that does not declare image references: it
//!   refuses a reference as an unsupported image;
//! - for a host that does, [`check_unresolved_images`] before resolution,
//!   [`resolve_images`] to decode inline images and resolve references in
//!   order, and [`check_resolved_images`] to run the inline checks on every
//!   image's bytes, with the same refusals and `detail.field` paths.

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
/// image formats, so this is a runner's check, not [`check_image`]'s. A
/// runner applies it to a resolved reference's bytes as to an inline
/// image's.
pub const IMAGE_MAX_PIXELS: u64 = 16_000_000;

/// Longest image reference handle, in characters.
pub const IMAGE_HANDLE_MAX_LEN: usize = 128;

/// Why a host that does not declare image references refuses one: it
/// cannot read the bytes, so to it a reference is an unsupported image.
pub const REFERENCE_UNSUPPORTED: &str =
    "this module does not resolve image references; send the image's bytes";

/// Standard base64, padded or not. Trailing bits that a canonical encoding
/// would leave zero are refused, so one byte string has one accepted
/// spelling per padding choice.
const LENIENT: GeneralPurpose = GeneralPurpose::new(
    &alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Whether `handle` is 1 to [`IMAGE_HANDLE_MAX_LEN`] characters of
/// `[A-Za-z0-9._:-]`.
pub fn is_image_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= IMAGE_HANDLE_MAX_LEN
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-'))
}

/// An image in its object form: `{content_type, base64}`.
///
/// Closed: a member other than these two is refused when decoding, so an
/// object that also carries `handle` is neither this nor an
/// [`ImageReference`], and is refused rather than silently read as one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
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

    /// The object form of decoded bytes, as standard padded base64: how a
    /// host forwards a resolved reference to a provider that reads only
    /// bytes.
    pub fn from_decoded(image: &DecodedImage) -> Self {
        Self::new(image.content_type.clone(), STANDARD.encode(&image.bytes))
    }
}

/// A reference to an image another module holds: `{handle}`.
///
/// The handle is opaque to every module but the one that issued it: 1 to
/// [`IMAGE_HANDLE_MAX_LEN`] characters of `[A-Za-z0-9._:-]`
/// ([`is_image_handle`]). The host resolves it through the module that
/// issued it. Closed like [`InlineImage`], so each JSON shape decodes as
/// exactly one variant of [`Image`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ImageReference {
    pub handle: String,
}

impl ImageReference {
    pub fn new(handle: impl Into<String>) -> Self {
        Self {
            handle: handle.into(),
        }
    }

    /// The handle's grammar checked, or why it is malformed.
    pub fn check(&self) -> Result<(), String> {
        if is_image_handle(&self.handle) {
            Ok(())
        } else {
            Err("an image handle is 1-128 characters of [A-Za-z0-9._:-]".into())
        }
    }
}

/// An image: a `data:` URL string, `{content_type, base64}`, or a reference
/// `{handle}`. Remote URLs are never accepted.
///
/// Untagged; each JSON shape matches one variant only, because both object
/// forms are closed. Non-exhaustive, so a match outside this crate needs a
/// wildcard arm and a later variant is not a breaking change.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum Image {
    DataUrl(String),
    Inline(InlineImage),
    Reference(ImageReference),
}

/// The canonical form of an image, as batch identity hashes it:
/// `{content_type, base64}` for an image that carries its bytes, `{handle}`
/// for a reference. Untagged, so it encodes as exactly that object.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum CanonicalImage {
    Inline(InlineImage),
    Reference(ImageReference),
}

/// An image's content type and decoded bytes: an inline image decoded, or
/// a reference resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedImage {
    pub content_type: String,
    pub bytes: Vec<u8>,
}

impl DecodedImage {
    /// The bytes a host resolved a reference to, with the content type the
    /// issuing module states for them.
    pub fn new(content_type: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            content_type: content_type.into(),
            bytes,
        }
    }
}

impl Image {
    /// The reference, when this image is one.
    pub fn reference(&self) -> Option<&ImageReference> {
        match self {
            Self::Reference(reference) => Some(reference),
            _ => None,
        }
    }

    /// The image's size in decoded bytes, estimated from its base64 length.
    /// Exact for well-formed base64; [`Image::decode`] is the check.
    ///
    /// `None` for a reference: its bytes are not in the request, and only
    /// the host's resolution can size them. `None` is not zero, so an
    /// estimate cannot count a reference as free by mistake.
    pub fn approx_bytes(&self) -> Option<usize> {
        let payload = match self {
            Self::DataUrl(url) => url.split_once(',').map_or("", |(_, data)| data),
            Self::Inline(inline) => &inline.base64,
            Self::Reference(_) => return None,
        };
        Some(payload.trim_end_matches('=').len() * 3 / 4)
    }

    /// The content type and decoded bytes, or why the image is malformed.
    /// A `data:` URL must be `data:<content type>;base64,<payload>`. The
    /// content type is not checked against a format list here.
    ///
    /// `None` for a reference, which has no bytes to decode: resolve it
    /// ([`resolve_images`]). The `Option` keeps "no bytes here" apart from
    /// both a malformed image (`Some(Err)`) and an empty one, so no caller
    /// can treat a reference as decoded or as refused by accident.
    pub fn decode(&self) -> Option<Result<DecodedImage, String>> {
        let (content_type, payload) = match self {
            Self::DataUrl(url) => {
                let Some(rest) = url.strip_prefix("data:") else {
                    return Some(Err(
                        "an image string must be a data: URL; remote URLs are refused".into(),
                    ));
                };
                let Some((media, payload)) = rest.split_once(',') else {
                    return Some(Err("a data: URL has no payload".into()));
                };
                let Some(content_type) = media.strip_suffix(";base64") else {
                    return Some(Err(
                        "a data: URL is data:<content type>;base64,<payload>".into()
                    ));
                };
                (content_type, payload)
            }
            Self::Inline(inline) => (inline.content_type.as_str(), inline.base64.as_str()),
            Self::Reference(_) => return None,
        };
        Some(decode_payload(content_type, payload))
    }

    /// The canonical form batch identity hashes: for an image with bytes,
    /// the content type and the decoded bytes as standard padded base64;
    /// for a reference, `{handle}` as sent, never the bytes it resolves to.
    /// The error says why the image (or the handle) is malformed.
    pub fn canonical(&self) -> Result<CanonicalImage, String> {
        if let Self::Reference(reference) = self {
            reference.check()?;
            return Ok(CanonicalImage::Reference(reference.clone()));
        }
        let decoded = self
            .decode()
            .ok_or_else(|| "the image carries no bytes".to_owned())??;
        Ok(CanonicalImage::Inline(InlineImage::from_decoded(&decoded)))
    }
}

fn decode_payload(content_type: &str, payload: &str) -> Result<DecodedImage, String> {
    let bytes = LENIENT
        .decode(payload)
        .map_err(|e| format!("the image's base64 does not decode: {e}"))?;
    if bytes.is_empty() {
        return Err("an image has no bytes".into());
    }
    Ok(DecodedImage::new(content_type, bytes))
}

/// The `image_reference` group of a role's `role.describe` answer. Its
/// presence declares that the host resolves image references (`RULES.md`
/// §13.1); a host that does not resolve them leaves the group out, and a
/// caller sends a reference only to a host that states it. It has no
/// members yet; later ones are additive, and decoding tolerates members
/// this crate does not know.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ImageReferenceSupport {}

impl ImageReferenceSupport {
    pub fn new() -> Self {
        Self {}
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

/// The content type and size checks every image's bytes get, whether the
/// request carried them or the host resolved them from a reference.
fn check_content_type(content_type: &str, limits: &ImageLimits) -> Result<(), String> {
    if limits.formats.iter().any(|format| format == content_type) {
        Ok(())
    } else {
        Err(format!("unsupported image content type {content_type}"))
    }
}

fn check_size(size: u64, limits: &ImageLimits) -> Result<u64, String> {
    if size > limits.max_bytes_each {
        return Err(format!(
            "an image is at most {} bytes",
            limits.max_bytes_each
        ));
    }
    Ok(size)
}

/// Check one image's form, content type, base64 and decoded size against
/// `limits`, returning its decoded size. The error says why it is refused.
///
/// A reference is refused ([`REFERENCE_UNSUPPORTED`]): it has no bytes to
/// check. A host that declares image references checks a reference's
/// handle with [`check_unresolved_images`] and its resolved bytes with
/// [`check_decoded_with`] instead.
pub fn check_image_with(image: &Image, limits: &ImageLimits) -> Result<u64, String> {
    let content_type = match image {
        Image::Reference(_) => return Err(REFERENCE_UNSUPPORTED.into()),
        Image::DataUrl(url) => {
            if !url.starts_with("data:") {
                return Err("an image string must be a data: URL; remote URLs are refused".into());
            }
            url.split_once(',')
                .map(|(media, _)| media["data:".len()..].split(';').next().unwrap_or(""))
                .ok_or_else(|| "a data: URL has no payload".to_owned())?
        }
        Image::Inline(inline) => inline.content_type.as_str(),
    };
    check_content_type(content_type, limits)?;
    let decoded = image
        .decode()
        .ok_or_else(|| REFERENCE_UNSUPPORTED.to_owned())??;
    check_size(decoded.bytes.len() as u64, limits)
}

/// Check one image's bytes, decoded from the request or resolved from a
/// reference, exactly as [`check_image_with`] checks an inline image: its
/// content type against `limits.formats`, that it has bytes, and its size
/// against `limits.max_bytes_each`. Returns its size; the error says why it
/// is refused, in the words an inline image gets.
pub fn check_decoded_with(image: &DecodedImage, limits: &ImageLimits) -> Result<u64, String> {
    check_content_type(&image.content_type, limits)?;
    if image.bytes.is_empty() {
        return Err("an image has no bytes".into());
    }
    check_size(image.bytes.len() as u64, limits)
}

/// Check one image against Workers AI's limits ([`ImageLimits::workers_ai`]).
/// A reference is refused, as [`check_image_with`] refuses it.
pub fn check_image(image: &Image) -> Result<(), String> {
    check_image_with(image, &ImageLimits::workers_ai(u64::MAX)).map(|_| ())
}

/// The item's image count against `limits.max`, `None` when the model
/// takes no images. Returns the limits when the model takes images.
fn check_count<'a>(
    model: &str,
    field: &str,
    count: usize,
    limits: Option<&'a ImageLimits>,
) -> Result<Option<&'a ImageLimits>, Refusal> {
    let max = limits.map_or(0, |limits| limits.max);
    if count as u64 > max {
        return Err(Refusal::over_limit(
            field,
            max,
            if limits.is_none() {
                format!("{model} takes no images")
            } else {
                format!("{count} images; at most {max}")
            },
        ));
    }
    Ok(limits)
}

fn check_total(field: &str, total: u64, limits: &ImageLimits) -> Result<(), Refusal> {
    if total > limits.max_bytes_total {
        return Err(Refusal::over_limit(
            field,
            limits.max_bytes_total,
            format!("{total} image bytes; at most {}", limits.max_bytes_total),
        ));
    }
    Ok(())
}

fn image_field(field: &str, j: usize) -> String {
    format!("{field}[{j}]")
}

/// Check an item's images: their count against `limits.max` (`None` when
/// the model takes no images), each image ([`check_image_with`]), and their
/// decoded total. `field` is the item's `images` path (`items[i].images`);
/// a refusal names it, or `field[j]` for one image.
///
/// This is the check of a host that does not declare image references: a
/// reference is refused `invalid_params` naming `field[j]`, as an
/// unsupported image.
pub fn check_images(
    model: &str,
    field: &str,
    images: &[Image],
    limits: Option<&ImageLimits>,
) -> Result<(), Refusal> {
    let Some(limits) = check_count(model, field, images.len(), limits)? else {
        return Ok(());
    };
    let mut total: u64 = 0;
    for (j, image) in images.iter().enumerate() {
        let size = check_image_with(image, limits)
            .map_err(|why| Refusal::invalid_params(image_field(field, j), why))?;
        total = total.saturating_add(size);
    }
    check_total(field, total, limits)
}

/// For a host that declares image references, the checks that need no
/// reference's bytes, run before resolution: the count, each reference's
/// handle ([`ImageReference::check`]), and each inline image
/// ([`check_image_with`]). The sizes of the whole item wait for
/// [`check_resolved_images`]. Refusals name `field` or `field[j]`, as
/// [`check_images`] does.
pub fn check_unresolved_images(
    model: &str,
    field: &str,
    images: &[Image],
    limits: Option<&ImageLimits>,
) -> Result<(), Refusal> {
    let Some(limits) = check_count(model, field, images.len(), limits)? else {
        return Ok(());
    };
    for (j, image) in images.iter().enumerate() {
        let checked = match image.reference() {
            Some(reference) => reference.check(),
            None => check_image_with(image, limits).map(|_| ()),
        };
        checked.map_err(|why| Refusal::invalid_params(image_field(field, j), why))?;
    }
    Ok(())
}

/// Why the host could not resolve a reference. Each maps to one admission
/// refusal naming the image ([`ResolveFailure::refusal`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ResolveFailure {
    /// The handle is unknown, expired, or not usable by this caller. One
    /// answer for all three, so a refusal never tells a caller whether a
    /// handle it may not use exists: `image_unavailable`, permanent.
    Unavailable,
    /// The module that issued the handle is unreachable or timed out:
    /// `image_source_unavailable`, transient, re-sent after
    /// `retry_after_ms`.
    SourceUnavailable { retry_after_ms: u64 },
}

impl ResolveFailure {
    /// The admission refusal for this failure, naming the image's `field`
    /// (`items[i].images[j]`).
    pub fn refusal(&self, field: impl Into<String>) -> Refusal {
        match *self {
            Self::Unavailable => Refusal::image_unavailable(field),
            Self::SourceUnavailable { retry_after_ms } => {
                Refusal::image_source_unavailable(field, retry_after_ms)
            }
        }
    }
}

/// Every image of an item as bytes, in order: an inline image decoded, a
/// reference resolved by `resolve`, which the host backs with the module
/// that issued the handle. `resolve` is called once per reference, in
/// order, and only for a well-formed handle. `field` is the item's `images`
/// path; a refusal names `field[j]`:
///
/// - a malformed inline image or handle: `invalid_params`;
/// - a failed resolution: [`ResolveFailure::refusal`].
///
/// A host that resolves asynchronously resolves first and passes a
/// `resolve` that looks the results up. Then [`check_resolved_images`]
/// runs the inline checks on the bytes this returns.
pub fn resolve_images(
    field: &str,
    images: &[Image],
    mut resolve: impl FnMut(&ImageReference) -> Result<DecodedImage, ResolveFailure>,
) -> Result<Vec<DecodedImage>, Refusal> {
    let mut decoded = Vec::with_capacity(images.len());
    for (j, image) in images.iter().enumerate() {
        let bytes = match (image.decode(), image.reference()) {
            (Some(inline), _) => {
                inline.map_err(|why| Refusal::invalid_params(image_field(field, j), why))?
            }
            (None, Some(reference)) => {
                reference
                    .check()
                    .map_err(|why| Refusal::invalid_params(image_field(field, j), why))?;
                resolve(reference).map_err(|failure| failure.refusal(image_field(field, j)))?
            }
            (None, None) => {
                return Err(Refusal::invalid_params(
                    image_field(field, j),
                    "the image carries no bytes",
                ))
            }
        };
        decoded.push(bytes);
    }
    Ok(decoded)
}

/// The checks an inline image gets, run on every image's bytes after the
/// host resolved the item's references ([`resolve_images`]): the count,
/// each image's content type and size ([`check_decoded_with`]), and the
/// item's total. `images[j]` is the bytes of the request's image `j`, so a
/// refusal names the same `field` or `field[j]` an inline image would get.
pub fn check_resolved_images(
    model: &str,
    field: &str,
    images: &[DecodedImage],
    limits: Option<&ImageLimits>,
) -> Result<(), Refusal> {
    let Some(limits) = check_count(model, field, images.len(), limits)? else {
        return Ok(());
    };
    let mut total: u64 = 0;
    for (j, image) in images.iter().enumerate() {
        let size = check_decoded_with(image, limits)
            .map_err(|why| Refusal::invalid_params(image_field(field, j), why))?;
        total = total.saturating_add(size);
    }
    check_total(field, total, limits)
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
        assert_eq!(
            url.canonical().unwrap(),
            CanonicalImage::Inline(InlineImage::new("image/png", "AAECAw=="))
        );
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

    fn decode(json: &str) -> Result<Image, serde_json::Error> {
        serde_json::from_str(json)
    }

    #[test]
    fn each_json_shape_decodes_as_one_variant_and_mixed_shapes_as_none() {
        assert_eq!(
            decode(r#"{"handle":"shot:1"}"#).unwrap(),
            Image::Reference(ImageReference::new("shot:1"))
        );
        assert_eq!(
            decode(r#"{"content_type":"image/png","base64":"AAAA"}"#).unwrap(),
            Image::Inline(InlineImage::new("image/png", "AAAA"))
        );
        assert_eq!(
            decode(r#""data:image/png;base64,AAAA""#).unwrap(),
            Image::DataUrl("data:image/png;base64,AAAA".into())
        );
        // Before the object forms were closed, the first of these decoded
        // as an inline image and dropped the handle.
        for mixed in [
            r#"{"content_type":"image/png","base64":"AAAA","handle":"shot:1"}"#,
            r#"{"content_type":"image/png","handle":"shot:1"}"#,
            r#"{"base64":"AAAA","handle":"shot:1"}"#,
            r#"{"handle":"shot:1","expires":5}"#,
            r#"{"content_type":"image/png","base64":"AAAA","url":"x"}"#,
            r#"{}"#,
        ] {
            assert!(decode(mixed).is_err(), "{mixed} decoded");
        }
        let reference = Image::Reference(ImageReference::new("shot:1"));
        assert_eq!(
            serde_json::to_string(&reference).unwrap(),
            r#"{"handle":"shot:1"}"#
        );
    }

    #[test]
    fn handles_are_1_to_128_characters_of_a_closed_set() {
        for good in ["a", "shot:01.png", "A-Z_09", &"x".repeat(128)] {
            assert!(is_image_handle(good), "{good}");
        }
        for bad in ["", "a b", "a/b", "é", "a\n", &"x".repeat(129)] {
            assert!(!is_image_handle(bad), "{bad:?}");
            assert!(Image::Reference(ImageReference::new(bad))
                .canonical()
                .is_err());
        }
    }

    #[test]
    fn a_reference_has_no_bytes_and_its_canonical_form_is_the_handle() {
        let reference = Image::Reference(ImageReference::new("shot:1"));
        assert_eq!(reference.decode(), None);
        assert_eq!(reference.approx_bytes(), None);
        assert_eq!(
            Image::Inline(InlineImage::new("image/png", "AAAA")).approx_bytes(),
            Some(3)
        );
        assert_eq!(
            reference.canonical().unwrap(),
            CanonicalImage::Reference(ImageReference::new("shot:1"))
        );
        assert_eq!(
            serde_json::to_string(&reference.canonical().unwrap()).unwrap(),
            r#"{"handle":"shot:1"}"#
        );
    }

    #[test]
    fn a_host_without_references_refuses_one_as_an_unsupported_image() {
        let limits = ImageLimits::workers_ai(4);
        let reference = Image::Reference(ImageReference::new("shot:1"));
        assert_eq!(check_image(&reference), Err(REFERENCE_UNSUPPORTED.into()));
        let pixel = Image::Inline(InlineImage::new("image/png", "AAAA"));
        let refusal =
            check_images("m", "items[2].images", &[pixel, reference], Some(&limits)).unwrap_err();
        assert_eq!(refusal.code, crate::errors::INVALID_PARAMS);
        assert_eq!(refusal.field(), Some("items[2].images[1]"));
    }

    #[test]
    fn a_resolved_reference_gets_the_inline_checks_at_its_own_path() {
        let limits = ImageLimits::new(3, 3, 5, vec![content_types::PNG.into()]);
        let field = "items[1].images";
        let images = [
            Image::Inline(InlineImage::new("image/png", "AAAA")),
            Image::Reference(ImageReference::new("big")),
            Image::Reference(ImageReference::new("gif")),
        ];
        assert_eq!(
            check_unresolved_images("m", field, &images, Some(&limits)),
            Ok(())
        );
        let mut asked = Vec::new();
        let resolved = resolve_images(field, &images, |reference| {
            asked.push(reference.handle.clone());
            Ok(match reference.handle.as_str() {
                "big" => DecodedImage::new("image/png", vec![0; 4]),
                _ => DecodedImage::new("image/gif", vec![0; 1]),
            })
        })
        .unwrap();
        assert_eq!(asked, ["big", "gif"]);
        assert_eq!(resolved[0], DecodedImage::new("image/png", vec![0; 3]));
        let oversized = check_resolved_images("m", field, &resolved, Some(&limits)).unwrap_err();
        let inline_oversized = check_images(
            "m",
            field,
            &[
                images[0].clone(),
                Image::Inline(InlineImage::from_decoded(&resolved[1])),
            ],
            Some(&limits),
        )
        .unwrap_err();
        assert_eq!(oversized, inline_oversized);
        assert_eq!(oversized.field(), Some("items[1].images[1]"));
        let wrong_type =
            check_resolved_images("m", field, &[resolved[2].clone()], Some(&limits)).unwrap_err();
        assert_eq!(
            wrong_type.message,
            "unsupported image content type image/gif"
        );
        assert_eq!(wrong_type.field(), Some("items[1].images[0]"));
        let empty = DecodedImage::new("image/png", Vec::new());
        let refusal = check_resolved_images("m", field, &[empty], Some(&limits)).unwrap_err();
        assert_eq!(refusal.message, "an image has no bytes");
        let small = DecodedImage::new("image/png", vec![0; 3]);
        let refusal =
            check_resolved_images("m", field, &[small.clone(), small], Some(&limits)).unwrap_err();
        assert_eq!(refusal.field(), Some(field));
        assert_eq!(refusal.detail.limit, Some(5));
    }

    #[test]
    fn failed_resolutions_and_malformed_handles_are_refused_at_the_image() {
        use crate::errors::{
            ErrorClass, IMAGE_SOURCE_UNAVAILABLE, IMAGE_UNAVAILABLE, INVALID_PARAMS,
        };
        let field = "items[0].images";
        let images = [Image::Reference(ImageReference::new("gone"))];
        let unavailable =
            resolve_images(field, &images, |_| Err(ResolveFailure::Unavailable)).unwrap_err();
        assert_eq!(unavailable.code, IMAGE_UNAVAILABLE);
        assert_eq!(unavailable.detail.class, ErrorClass::Permanent);
        assert_eq!(unavailable.field(), Some("items[0].images[0]"));
        let down = resolve_images(field, &images, |_| {
            Err(ResolveFailure::SourceUnavailable {
                retry_after_ms: 750,
            })
        })
        .unwrap_err();
        assert_eq!(down.code, IMAGE_SOURCE_UNAVAILABLE);
        assert_eq!(down.detail.class, ErrorClass::Transient);
        assert_eq!(down.detail.retry_after_ms, Some(750));
        assert_eq!(down.field(), Some("items[0].images[0]"));
        let malformed = [Image::Reference(ImageReference::new("no spaces"))];
        let refusal = resolve_images(field, &malformed, |_| panic!("not resolved")).unwrap_err();
        assert_eq!(refusal.code, INVALID_PARAMS);
        assert_eq!(refusal.field(), Some("items[0].images[0]"));
        let refusal =
            check_unresolved_images("m", field, &malformed, Some(&ImageLimits::workers_ai(1)))
                .unwrap_err();
        assert_eq!(refusal.field(), Some("items[0].images[0]"));
        // A model that takes no images refuses a reference by count, as it
        // refuses an inline image.
        let refusal = check_unresolved_images("m", field, &images, None).unwrap_err();
        assert_eq!(refusal.field(), Some(field));
        assert_eq!(refusal.detail.limit, Some(0));
    }
}
