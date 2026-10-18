//! The shared ChatGPT/Codex policy vectors, compiled in (feature `test-vectors`).
//!
//! The JSON files live in this crate's `test-vectors/codex-policy/` and ship in
//! its package so clients can test against these exact bytes. `SHA256SUMS`
//! pins their contents, and `FILES` lists every JSON vector file.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "codex-policy";

/// Account-id resolution cases, as a JSON document with a `vectors` array.
pub const ACCOUNT_IDS_JSON: &str = include_str!("../test-vectors/codex-policy/account_ids.json");

/// HTTP refusal-classification cases, as a JSON document with a `vectors` array.
pub const RESPONSES_JSON: &str = include_str!("../test-vectors/codex-policy/responses.json");

/// `sha256sum`-format checksums of the vector files above.
pub const SHA256SUMS: &str = include_str!("../test-vectors/codex-policy/SHA256SUMS");

/// Every vector file, as `(file name, contents)`.
pub const FILES: &[(&str, &str)] = &[
    ("account_ids.json", ACCOUNT_IDS_JSON),
    ("responses.json", RESPONSES_JSON),
];
