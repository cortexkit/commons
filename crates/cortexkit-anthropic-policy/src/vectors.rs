//! The Claude OAuth wire test vectors, compiled in (feature `test-vectors`).
//!
//! Each case fixes a request a client receives from its host and the exact
//! request it must send on the OAuth `POST /v1/messages` route: headers in
//! wire order, body bytes, the billing header and `cch`. Every client that
//! sends Claude Code-shaped requests tests against these same bytes, so the
//! clients cannot drift apart. The files live in this crate's
//! `test-vectors/claude-oauth-wire/` and ship in its package; `SHA256SUMS`
//! pins their bytes and `README.md` describes each case and every value held
//! fixed.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "claude-oauth-wire";

/// The vector cases, as a JSON document with `schema`, `generator`, `fixed`,
/// `primitives` and `cases`.
pub const VECTORS_JSON: &str = include_str!("../test-vectors/claude-oauth-wire/vectors.json");

/// The case and field guide for [`VECTORS_JSON`].
pub const README: &str = include_str!("../test-vectors/claude-oauth-wire/README.md");

/// `sha256sum`-format checksums of the files above.
pub const SHA256SUMS: &str = include_str!("../test-vectors/claude-oauth-wire/SHA256SUMS");

/// Every vector file, as `(file name, contents)`.
pub const FILES: &[(&str, &str)] = &[("vectors.json", VECTORS_JSON), ("README.md", README)];
