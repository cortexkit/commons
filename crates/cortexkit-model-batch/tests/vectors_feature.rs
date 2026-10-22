//! The public `vectors` module carries the exact bytes of every shipped file.

use cortexkit_model_batch::vectors;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn compiled_in_vectors_match_the_files() {
    let dir = format!(
        "{}/test-vectors/{}",
        env!("CARGO_MANIFEST_DIR"),
        vectors::DIR
    );
    let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS")
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = vectors::FILES
        .iter()
        .map(|(name, _)| name.to_string())
        .collect();
    listed.sort();
    assert_eq!(on_disk, listed);

    for (name, contents) in vectors::FILES {
        let bytes = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
        assert_eq!(*contents, bytes, "{name}");
        assert_eq!(vectors::file(name), Some(*contents));
    }
    let sums = std::fs::read_to_string(format!("{dir}/SHA256SUMS")).unwrap();
    assert_eq!(vectors::SHA256SUMS, sums);
    let names: Vec<&str> = vectors::SHA256SUMS
        .lines()
        .map(|line| line.split_once("  ").unwrap().1)
        .collect();
    let in_order: Vec<&str> = vectors::FILES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, in_order, "SHA256SUMS lists FILES in order");
}

#[test]
fn files_match_sha256sums() {
    let checksums: BTreeMap<&str, &str> = vectors::SHA256SUMS
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").expect("sha256sum line");
            (name, digest)
        })
        .collect();
    let listed: BTreeSet<&str> = vectors::FILES.iter().map(|(name, _)| *name).collect();
    assert_eq!(checksums.len(), vectors::SHA256SUMS.lines().count());
    assert_eq!(checksums.keys().copied().collect::<BTreeSet<_>>(), listed);

    for (name, contents) in vectors::FILES {
        assert_eq!(
            format!("{:x}", Sha256::digest(contents.as_bytes())),
            checksums[name],
            "{name}: SHA-256 mismatch"
        );
    }
}
