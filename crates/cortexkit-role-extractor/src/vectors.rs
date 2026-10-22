//! The `extractor/v1` vectors, compiled in with `test-vectors`.
//!
//! The files live in this crate's `test-vectors/extractor-v1/` directory
//! and ship in its package, so a module's tests can use the same bytes as
//! the wire crate's own vector tests.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "extractor-v1";

/// The `admission-refusals.json` vector cases.
pub const ADMISSION_REFUSALS_JSON: &str =
    include_str!("../test-vectors/extractor-v1/admission-refusals.json");
/// The `answer-too-large.json` vector cases.
pub const ANSWER_TOO_LARGE_JSON: &str =
    include_str!("../test-vectors/extractor-v1/answer-too-large.json");
/// The `field-paths.json` vector cases.
pub const FIELD_PATHS_JSON: &str = include_str!("../test-vectors/extractor-v1/field-paths.json");
/// The `identity.json` vector cases.
pub const IDENTITY_JSON: &str = include_str!("../test-vectors/extractor-v1/identity.json");
/// The `provider-replies.json` vector cases: Clairic's reply bodies from
/// a live probe of 2026-10-22, byte for byte, each with the item a
/// module answers with.
pub const PROVIDER_REPLIES_JSON: &str =
    include_str!("../test-vectors/extractor-v1/provider-replies.json");
/// The `reply-size.json` vector cases.
pub const REPLY_SIZE_JSON: &str = include_str!("../test-vectors/extractor-v1/reply-size.json");
/// The `requests.json` vector cases.
pub const REQUESTS_JSON: &str = include_str!("../test-vectors/extractor-v1/requests.json");
/// The `role-describe.json` vector cases.
pub const ROLE_DESCRIBE_JSON: &str =
    include_str!("../test-vectors/extractor-v1/role-describe.json");
/// The `spend.json` vector cases.
pub const SPEND_JSON: &str = include_str!("../test-vectors/extractor-v1/spend.json");

/// Every vector file as `(file name, contents)`, in `SHA256SUMS` order.
pub const FILES: &[(&str, &str)] = &[
    ("admission-refusals.json", ADMISSION_REFUSALS_JSON),
    ("answer-too-large.json", ANSWER_TOO_LARGE_JSON),
    ("field-paths.json", FIELD_PATHS_JSON),
    ("identity.json", IDENTITY_JSON),
    ("provider-replies.json", PROVIDER_REPLIES_JSON),
    ("reply-size.json", REPLY_SIZE_JSON),
    ("requests.json", REQUESTS_JSON),
    ("role-describe.json", ROLE_DESCRIBE_JSON),
    ("spend.json", SPEND_JSON),
];

/// `sha256sum` output for every file in [`FILES`].
pub const SHA256SUMS: &str = include_str!("../test-vectors/extractor-v1/SHA256SUMS");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}
