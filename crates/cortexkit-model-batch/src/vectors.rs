//! This crate's vectors, compiled in with `test-vectors`.
//!
//! The files live in this crate's `test-vectors/model-batch-v1/` directory
//! and ship in its package, so a role crate or a module can test against the
//! same bytes as this crate's own vector tests.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "model-batch-v1";

/// The `estimates.json` vector cases.
pub const ESTIMATES_JSON: &str = include_str!("../test-vectors/model-batch-v1/estimates.json");
/// The `images.json` vector cases.
pub const IMAGES_JSON: &str = include_str!("../test-vectors/model-batch-v1/images.json");
/// The `status-mapping.json` vector cases.
pub const STATUS_MAPPING_JSON: &str =
    include_str!("../test-vectors/model-batch-v1/status-mapping.json");
/// The `vocabulary.json` vector cases.
pub const VOCABULARY_JSON: &str = include_str!("../test-vectors/model-batch-v1/vocabulary.json");

/// Every vector file as `(file name, contents)`, in `SHA256SUMS` order.
pub const FILES: &[(&str, &str)] = &[
    ("estimates.json", ESTIMATES_JSON),
    ("images.json", IMAGES_JSON),
    ("status-mapping.json", STATUS_MAPPING_JSON),
    ("vocabulary.json", VOCABULARY_JSON),
];

/// `sha256sum` output for every file in [`FILES`].
pub const SHA256SUMS: &str = include_str!("../test-vectors/model-batch-v1/SHA256SUMS");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}
