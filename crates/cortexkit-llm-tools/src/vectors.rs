//! Invented examples shared with callers and serving modules.

pub const CLASSIFY: &str = include_str!("../test-vectors/llm-tools-v1/classify.json");
pub const EXTRACT: &str = include_str!("../test-vectors/llm-tools-v1/extract.json");
pub const COMPLETE: &str = include_str!("../test-vectors/llm-tools-v1/complete.json");
pub const REFUSALS: &str = include_str!("../test-vectors/llm-tools-v1/refusals.json");
pub const REFUSAL_SPEND: &str = include_str!("../test-vectors/llm-tools-v1/refusal-spend.json");
pub const SHA256SUMS: &str = include_str!("../test-vectors/llm-tools-v1/SHA256SUMS");

/// Embedded files, in SHA256SUMS order.
pub const FILES: &[(&str, &str)] = &[
    ("classify.json", CLASSIFY),
    ("complete.json", COMPLETE),
    ("extract.json", EXTRACT),
    ("refusal-spend.json", REFUSAL_SPEND),
    ("refusals.json", REFUSALS),
];
