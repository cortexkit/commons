//! The shipped Claude OAuth wire vectors parse, carry no real credentials,
//! and match their own SHA256SUMS.

use cortexkit_anthropic_policy::vectors;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

#[test]
fn sha256sums_cover_every_shipped_file_and_match_its_bytes() {
    let mut listed = BTreeSet::new();
    for line in vectors::SHA256SUMS
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        let (digest, name) = line.split_once("  ").expect("sha256sum line");
        let (_, contents) = vectors::FILES
            .iter()
            .find(|(file, _)| *file == name)
            .unwrap_or_else(|| {
                panic!("SHA256SUMS names {name}, which vectors::FILES does not ship")
            });
        assert_eq!(
            format!("{:x}", Sha256::digest(contents.as_bytes())),
            digest,
            "{name} does not match its SHA256SUMS entry"
        );
        listed.insert(name.to_owned());
    }
    let shipped: BTreeSet<_> = vectors::FILES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    assert_eq!(listed, shipped, "every shipped file must have a checksum");
}

#[test]
fn cases_are_unique_labelled_and_use_placeholder_credentials() {
    let doc: serde_json::Value =
        serde_json::from_str(vectors::VECTORS_JSON).expect("vectors.json parses");
    let cases = doc["cases"].as_array().expect("cases array");
    assert!(!cases.is_empty());
    let mut ids = BTreeSet::new();
    for case in cases {
        let id = case["id"].as_str().expect("case id");
        assert!(ids.insert(id.to_owned()), "duplicate case id {id}");
        let source = case["source"].as_str().expect("case source");
        assert!(
            matches!(source, "captured" | "reconstructed"),
            "{id}: unknown source {source}"
        );
    }
    // A real OAuth access token would be a credential in a public package.
    for (start, _) in vectors::VECTORS_JSON.match_indices("sk-ant-") {
        let token: String = vectors::VECTORS_JSON[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        assert!(
            token.ends_with("PLACEHOLDER"),
            "non-placeholder token {token}"
        );
    }
}
