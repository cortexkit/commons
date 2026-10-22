//! Shared `classifier/v1` vectors, compiled in with `test-vectors`.
//!
//! The files live in this crate's `test-vectors/classifier-v1/` directory
//! and ship in its package. Consumers can enable this feature to test against
//! the same bytes as the wire crate's own vector tests.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "classifier-v1";

/// The `admission-refusals.json` vector cases.
pub const ADMISSION_REFUSALS_JSON: &str =
    include_str!("../test-vectors/classifier-v1/admission-refusals.json");
/// The `field-paths.json` vector cases.
pub const FIELD_PATHS_JSON: &str = include_str!("../test-vectors/classifier-v1/field-paths.json");
/// The `image-identity.json` vector cases.
pub const IMAGE_IDENTITY_JSON: &str =
    include_str!("../test-vectors/classifier-v1/image-identity.json");
/// The `item-errors.json` vector cases.
pub const ITEM_ERRORS_JSON: &str = include_str!("../test-vectors/classifier-v1/item-errors.json");
/// The `provider-examples.json` vector cases.
pub const PROVIDER_EXAMPLES_JSON: &str =
    include_str!("../test-vectors/classifier-v1/provider-examples.json");
/// The `question-choice.json` vector cases.
pub const QUESTION_CHOICE_JSON: &str =
    include_str!("../test-vectors/classifier-v1/question-choice.json");
/// The `question-noul.json` vector cases.
pub const QUESTION_NOUL_JSON: &str =
    include_str!("../test-vectors/classifier-v1/question-noul.json");
/// The `question-score.json` vector cases.
pub const QUESTION_SCORE_JSON: &str =
    include_str!("../test-vectors/classifier-v1/question-score.json");
/// The `replay-batch-id-reuse.json` vector cases.
pub const REPLAY_BATCH_ID_REUSE_JSON: &str =
    include_str!("../test-vectors/classifier-v1/replay-batch-id-reuse.json");
/// The `replay-retry.json` vector cases.
pub const REPLAY_RETRY_JSON: &str = include_str!("../test-vectors/classifier-v1/replay-retry.json");
/// The `role-describe.json` vector cases.
pub const ROLE_DESCRIBE_JSON: &str =
    include_str!("../test-vectors/classifier-v1/role-describe.json");
/// The `state-array.json` vector cases.
pub const STATE_ARRAY_JSON: &str = include_str!("../test-vectors/classifier-v1/state-array.json");
/// The `state-object.json` vector cases.
pub const STATE_OBJECT_JSON: &str = include_str!("../test-vectors/classifier-v1/state-object.json");

/// The `workers-ai-codes.json` vector cases.
pub const WORKERS_AI_CODES_JSON: &str =
    include_str!("../test-vectors/classifier-v1/workers-ai-codes.json");

/// Every vector file as `(file name, contents)`, in `SHA256SUMS` order.
pub const FILES: &[(&str, &str)] = &[
    ("admission-refusals.json", ADMISSION_REFUSALS_JSON),
    ("field-paths.json", FIELD_PATHS_JSON),
    ("image-identity.json", IMAGE_IDENTITY_JSON),
    ("item-errors.json", ITEM_ERRORS_JSON),
    ("provider-examples.json", PROVIDER_EXAMPLES_JSON),
    ("question-choice.json", QUESTION_CHOICE_JSON),
    ("question-noul.json", QUESTION_NOUL_JSON),
    ("question-score.json", QUESTION_SCORE_JSON),
    ("replay-batch-id-reuse.json", REPLAY_BATCH_ID_REUSE_JSON),
    ("replay-retry.json", REPLAY_RETRY_JSON),
    ("role-describe.json", ROLE_DESCRIBE_JSON),
    ("state-array.json", STATE_ARRAY_JSON),
    ("state-object.json", STATE_OBJECT_JSON),
    ("workers-ai-codes.json", WORKERS_AI_CODES_JSON),
];

/// `sha256sum` output for every file in [`FILES`].
pub const SHA256SUMS: &str = include_str!("../test-vectors/classifier-v1/SHA256SUMS");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}
