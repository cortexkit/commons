//! The runner compiles in copies of two shared vectors (`vectors/` in this
//! crate) so the published crate builds without the repository around it.
//! This test keeps each copy byte-identical to the repository's
//! `test-vectors/tool-provider-v1/` file, which is the one the wire crate's
//! own tests read. When a vector there changes, copy it here too.
//!
//! It needs the repository checkout, so it fails rather than skips when the
//! repository file is missing: a skipped check here would let the copies drift
//! unnoticed.

use std::path::Path;

const VENDORED: &[&str] = &["call-key.json", "schema-pin.json"];

#[test]
fn vendored_vectors_match_the_repository_vectors() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repository = crate_dir.join("../../test-vectors/tool-provider-v1");
    for name in VENDORED {
        let canonical = std::fs::read(repository.join(name))
            .unwrap_or_else(|e| panic!("reading the repository's {name}: {e}"));
        let copy = std::fs::read(crate_dir.join("vectors").join(name))
            .unwrap_or_else(|e| panic!("reading this crate's copy of {name}: {e}"));
        assert!(
            copy == canonical,
            "vectors/{name} differs from test-vectors/tool-provider-v1/{name}; copy it again"
        );
    }
}
