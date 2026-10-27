//! The item image shape, its validation and its canonical form: the shared
//! model-batch rules (`cortexkit-model-batch`, its `RULES.md` §13),
//! re-exported for `extractor/v1`. A model's limits are its
//! `role.describe` entry's `images` ([`ImageLimits`]). An image may also be
//! a reference to an image another module holds ([`ImageReference`], RULES
//! §13.1); a host that declares `image_reference` resolves it with
//! [`resolve_images`] and checks the bytes with [`check_resolved_images`].

pub use cortexkit_model_batch::image::{
    check_decoded_with, check_image, check_image_with, check_images, check_resolved_images,
    check_unresolved_images, content_types, is_image_handle, resolve_images, CanonicalImage,
    DecodedImage, Image, ImageLimits, ImageReference, InlineImage, ResolveFailure,
    IMAGES_MAX_TOTAL_BYTES, IMAGE_HANDLE_MAX_LEN, IMAGE_MAX_BYTES, IMAGE_MAX_PIXELS,
    REFERENCE_UNSUPPORTED,
};
