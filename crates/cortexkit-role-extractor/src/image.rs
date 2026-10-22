//! The item image shape, its validation and its canonical form: the shared
//! model-batch rules (`cortexkit-model-batch`, its `RULES.md` §13),
//! re-exported for `extractor/v1`. A model's limits are its
//! `role.describe` entry's `images` ([`ImageLimits`]).

pub use cortexkit_model_batch::image::{
    check_image, check_image_with, check_images, content_types, DecodedImage, Image, ImageLimits,
    InlineImage, IMAGES_MAX_TOTAL_BYTES, IMAGE_MAX_BYTES, IMAGE_MAX_PIXELS,
};
