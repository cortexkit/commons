//! Byte-for-byte shared vectors. Large payloads use the recipes in README.md.

pub const FILES: &[(&str, &str)] = &[
    (
        "README.md",
        include_str!("../test-vectors/media-v1/README.md"),
    ),
    (
        "budgets.json",
        include_str!("../test-vectors/media-v1/budgets.json"),
    ),
    (
        "chunks.json",
        include_str!("../test-vectors/media-v1/chunks.json"),
    ),
    (
        "describe.json",
        include_str!("../test-vectors/media-v1/describe.json"),
    ),
    (
        "hand-off.json",
        include_str!("../test-vectors/media-v1/hand-off.json"),
    ),
    (
        "kinds.json",
        include_str!("../test-vectors/media-v1/kinds.json"),
    ),
    (
        "packs.json",
        include_str!("../test-vectors/media-v1/packs.json"),
    ),
    (
        "realtime.json",
        include_str!("../test-vectors/media-v1/realtime.json"),
    ),
    (
        "refusals.json",
        include_str!("../test-vectors/media-v1/refusals.json"),
    ),
    (
        "trim-bias.json",
        include_str!("../test-vectors/media-v1/trim-bias.json"),
    ),
    (
        "usage-feed.json",
        include_str!("../test-vectors/media-v1/usage-feed.json"),
    ),
    (
        "usage.json",
        include_str!("../test-vectors/media-v1/usage.json"),
    ),
];
pub const SHA256SUMS: &str = include_str!("../test-vectors/media-v1/SHA256SUMS");

pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, contents)| *contents)
}
