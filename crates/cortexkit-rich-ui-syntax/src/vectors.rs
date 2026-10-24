//! Reference-scanning sample files compiled in with the `test-vectors` feature.
//!
//! The files live in this crate's `test-vectors/rich-ui-syntax-v1/` directory
//! and ship in its package. [`FILES`] exposes their exact contents, and
//! [`SHA256SUMS`] pins those bytes for consumers that need to test against the
//! same samples without a repository checkout.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "rich-ui-syntax-v1";

/// The original 27-case sample file, unchanged from its source.
pub const ORIGINAL_SAMPLES: &str =
    include_str!("../test-vectors/rich-ui-syntax-v1/refs-scan-samples-v1.json");

/// The extended sample file.
pub const EXTENDED_SAMPLES: &str =
    include_str!("../test-vectors/rich-ui-syntax-v1/refs-scan-samples-v1.1.json");

/// Streaming-hazard and diagnostic samples. Their cases may also pin the
/// exact `diagnostics` list: code, span and subject.
pub const STREAM_AND_DIAGNOSTIC_SAMPLES: &str =
    include_str!("../test-vectors/rich-ui-syntax-v1/refs-scan-samples-v1.2.json");

/// The sample-set guide.
pub const README: &str = include_str!("../test-vectors/rich-ui-syntax-v1/README.md");

/// Every vector file as `(file name, contents)`, in [`SHA256SUMS`] order.
pub const FILES: &[(&str, &str)] = &[
    ("refs-scan-samples-v1.json", ORIGINAL_SAMPLES),
    ("refs-scan-samples-v1.1.json", EXTENDED_SAMPLES),
    ("refs-scan-samples-v1.2.json", STREAM_AND_DIAGNOSTIC_SAMPLES),
    ("README.md", README),
];

/// `sha256sum` output pinning every file in the vectors directory except itself.
pub const SHA256SUMS: &str = include_str!("../test-vectors/rich-ui-syntax-v1/SHA256SUMS");

/// The contents of vector file `name`, if it is listed in [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}
