//! Shared `tool-provider/v1` vectors, compiled in with `test-vectors`.
//!
//! The files live in this crate's `test-vectors/tool-provider-v1/` directory
//! and ship in its package. Consumers can enable this feature to test against
//! the same bytes as the wire crate and conformance suite.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "tool-provider-v1";

/// The `call-key.json` vector cases.
pub const CALL_KEY_JSON: &str = include_str!("../test-vectors/tool-provider-v1/call-key.json");
/// The `call-modes.json` vector cases.
pub const CALL_MODES_JSON: &str = include_str!("../test-vectors/tool-provider-v1/call-modes.json");
/// The `catalog-answers.json` vector cases.
pub const CATALOG_ANSWERS_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/catalog-answers.json");
/// The `catalog-capabilities.json` vector cases.
pub const CATALOG_CAPABILITIES_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/catalog-capabilities.json");
/// The `catalog-requests.json` vector cases.
pub const CATALOG_REQUESTS_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/catalog-requests.json");
/// The `catalog-schemas.json` vector cases.
pub const CATALOG_SCHEMAS_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/catalog-schemas.json");
/// The `composition-digest.json` vector cases.
pub const COMPOSITION_DIGEST_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/composition-digest.json");
/// The `content-v1.json` vector cases.
pub const CONTENT_V1_JSON: &str = include_str!("../test-vectors/tool-provider-v1/content-v1.json");
/// The `emits-events.json` vector cases.
pub const EMITS_EVENTS_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/emits-events.json");
/// The `held-reply-v1.json` vector cases.
pub const HELD_REPLY_V1_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/held-reply-v1.json");
/// The `late-results.json` vector cases.
pub const LATE_RESULTS_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/late-results.json");
/// The `reply-deadline.json` vector cases.
pub const REPLY_DEADLINE_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/reply-deadline.json");
/// The `role-describe.json` vector cases.
pub const ROLE_DESCRIBE_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/role-describe.json");
/// The `schema-digest.json` vector cases.
pub const SCHEMA_DIGEST_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/schema-digest.json");
/// The `schema-pin.json` vector cases.
pub const SCHEMA_PIN_JSON: &str = include_str!("../test-vectors/tool-provider-v1/schema-pin.json");
/// The `withdraw-answers.json` vector cases.
pub const WITHDRAW_ANSWERS_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/withdraw-answers.json");
/// A reply body as the bare answer object, and a refusal body with its
/// other fields inside `detail`.
pub const REPLY_SHAPE_JSON: &str = include_str!("../test-vectors/tool-provider-v1/reply-shape.json");
/// Person-only command declarations, calls, replies, scope table and late entries.
pub const USER_COMMANDS_JSON: &str =
    include_str!("../test-vectors/tool-provider-v1/user-commands.json");
/// The vectors' `README.md`.
pub const README: &str = include_str!("../test-vectors/tool-provider-v1/README.md");

/// Every vector file as `(file name, contents)`, in `SHA256SUMS` order.
pub const FILES: &[(&str, &str)] = &[
    ("call-key.json", CALL_KEY_JSON),
    ("call-modes.json", CALL_MODES_JSON),
    ("catalog-answers.json", CATALOG_ANSWERS_JSON),
    ("catalog-capabilities.json", CATALOG_CAPABILITIES_JSON),
    ("catalog-requests.json", CATALOG_REQUESTS_JSON),
    ("catalog-schemas.json", CATALOG_SCHEMAS_JSON),
    ("composition-digest.json", COMPOSITION_DIGEST_JSON),
    ("content-v1.json", CONTENT_V1_JSON),
    ("emits-events.json", EMITS_EVENTS_JSON),
    ("held-reply-v1.json", HELD_REPLY_V1_JSON),
    ("late-results.json", LATE_RESULTS_JSON),
    ("reply-deadline.json", REPLY_DEADLINE_JSON),
    ("role-describe.json", ROLE_DESCRIBE_JSON),
    ("schema-digest.json", SCHEMA_DIGEST_JSON),
    ("schema-pin.json", SCHEMA_PIN_JSON),
    ("withdraw-answers.json", WITHDRAW_ANSWERS_JSON),
    ("user-commands.json", USER_COMMANDS_JSON),
    ("reply-shape.json", REPLY_SHAPE_JSON),
    ("README.md", README),
];

/// `sha256sum` output for every file in [`FILES`].
pub const SHA256SUMS: &str = include_str!("../test-vectors/tool-provider-v1/SHA256SUMS");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}

#[cfg(test)]
pub(crate) fn load(name: &str) -> serde_json::Value {
    let text = file(name).unwrap_or_else(|| panic!("unknown tool-provider vector: {name}"));
    serde_json::from_str(text)
        .unwrap_or_else(|e| panic!("invalid tool-provider vector {name}: {e}"))
}
