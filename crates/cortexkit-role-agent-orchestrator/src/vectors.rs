//! Packaged `awaited-work/v1` wire examples.

pub const DIR: &str = "awaited-work-v1";
pub const FILES: &[(&str, &str)] = &[
    (
        "bounds.json",
        include_str!("../test-vectors/awaited-work-v1/bounds.json"),
    ),
    (
        "refusals.json",
        include_str!("../test-vectors/awaited-work-v1/refusals.json"),
    ),
    (
        "replies.json",
        include_str!("../test-vectors/awaited-work-v1/replies.json"),
    ),
    (
        "requests.json",
        include_str!("../test-vectors/awaited-work-v1/requests.json"),
    ),
    ("README.md", README),
];
pub const SHA256SUMS: &str = include_str!("../test-vectors/awaited-work-v1/SHA256SUMS");
pub const README: &str = include_str!("../test-vectors/awaited-work-v1/README.md");

pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}
