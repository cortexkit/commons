//! The contract's test vectors, compiled in (feature `test-vectors`).
//!
//! The files live in this crate's `test-vectors/agent-data-v1/` and ship
//! in its package, so a participant that depends on this crate by version
//! loads exactly the bytes this crate's own tests check. [`SHA256SUMS`]
//! pins every file's bytes; `README.md` ([`README`]) describes each file's
//! cases and how to read them.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "agent-data-v1";

/// Every vector file, as `(file name, contents)`, in the order
/// [`SHA256SUMS`] lists them.
pub const FILES: &[(&str, &str)] = &[
    (
        "requests.json",
        include_str!("../test-vectors/agent-data-v1/requests.json"),
    ),
    (
        "replies.json",
        include_str!("../test-vectors/agent-data-v1/replies.json"),
    ),
    (
        "refusals.json",
        include_str!("../test-vectors/agent-data-v1/refusals.json"),
    ),
    (
        "idempotency.json",
        include_str!("../test-vectors/agent-data-v1/idempotency.json"),
    ),
    (
        "steps.json",
        include_str!("../test-vectors/agent-data-v1/steps.json"),
    ),
    (
        "ref-sets.json",
        include_str!("../test-vectors/agent-data-v1/ref-sets.json"),
    ),
    (
        "op-refusals.json",
        include_str!("../test-vectors/agent-data-v1/op-refusals.json"),
    ),
    (
        "session-refs.json",
        include_str!("../test-vectors/agent-data-v1/session-refs.json"),
    ),
    (
        "README.md",
        include_str!("../test-vectors/agent-data-v1/README.md"),
    ),
    (
        "digest-mismatch.json",
        include_str!("../test-vectors/agent-data-v1/digest-mismatch.json"),
    ),
    (
        "diverged-reasons.json",
        include_str!("../test-vectors/agent-data-v1/diverged-reasons.json"),
    ),
    (
        "import-drops.json",
        include_str!("../test-vectors/agent-data-v1/import-drops.json"),
    ),
];

/// `sha256sum` output over [`FILES`]: one `<hex>  <name>` line per file.
pub const SHA256SUMS: &str = include_str!("../test-vectors/agent-data-v1/SHA256SUMS");

/// The vectors' `README.md`.
pub const README: &str = include_str!("../test-vectors/agent-data-v1/README.md");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}
