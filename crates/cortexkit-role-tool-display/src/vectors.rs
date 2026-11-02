//! Packaged vector bytes, available without a repository checkout.

/// The provider-owned views fixture, copied byte for byte into this package.
pub const VIEWS_JSON: &str = include_str!("../test-vectors/tool-display-v1/views.json");
/// Provider-owned view manifests with a matching tool definition, a mismatched
/// one, or none (an older provider's view).
pub const VIEW_TOOLS_JSON: &str = include_str!("../test-vectors/tool-display-v1/view-tools.json");

/// Every vector and its README, in checksum-manifest order.
pub const FILES: &[(&str, &str)] = &[
    (
        "canonical.json",
        include_str!("../test-vectors/tool-display-v1/canonical.json"),
    ),
    (
        "field-kinds.json",
        include_str!("../test-vectors/tool-display-v1/field-kinds.json"),
    ),
    (
        "records.json",
        include_str!("../test-vectors/tool-display-v1/records.json"),
    ),
    (
        "details.json",
        include_str!("../test-vectors/tool-display-v1/details.json"),
    ),
    (
        "writes.json",
        include_str!("../test-vectors/tool-display-v1/writes.json"),
    ),
    ("views.json", VIEWS_JSON),
    ("view-tools.json", VIEW_TOOLS_JSON),
    (
        "README.md",
        include_str!("../test-vectors/tool-display-v1/README.md"),
    ),
];

/// SHA-256 pins for every member of [`FILES`].
pub const SHA256SUMS: &str = include_str!("../test-vectors/tool-display-v1/SHA256SUMS");
