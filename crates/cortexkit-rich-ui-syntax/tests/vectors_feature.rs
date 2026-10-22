//! The shipped reference-scanning samples match their packaged checksums.

use cortexkit_rich_ui_syntax::vectors;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

#[test]
fn every_vector_file_on_disk_is_listed_with_its_bytes() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("test-vectors")
        .join(vectors::DIR);
    fn collect(dir: &std::path::Path, prefix: &str, names: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_str().unwrap());
            if entry.file_type().unwrap().is_dir() {
                collect(&entry.path(), &format!("{name}/"), names);
            } else if name != "SHA256SUMS" {
                names.push(name);
            }
        }
    }

    let mut on_disk = Vec::new();
    collect(&dir, "", &mut on_disk);
    on_disk.sort();
    let mut listed: Vec<String> = vectors::FILES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();
    listed.sort();
    assert_eq!(on_disk, listed);

    for (name, contents) in vectors::FILES {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        assert_eq!(contents.as_bytes(), bytes.as_slice(), "{name}");
        assert_eq!(vectors::file(name), Some(*contents));
    }
    assert_eq!(vectors::file("no-such-file.json"), None);
    assert_eq!(
        vectors::SHA256SUMS,
        std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap()
    );
}

#[test]
fn files_match_sha256sums() {
    let names: Vec<&str> = vectors::SHA256SUMS
        .lines()
        .map(|line| line.split_once("  ").expect("sha256sum line").1)
        .collect();
    let in_order: Vec<&str> = vectors::FILES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, in_order, "SHA256SUMS lists FILES in order");

    let checksums: BTreeMap<&str, &str> = vectors::SHA256SUMS
        .lines()
        .map(|line| {
            let (digest, name) = line.split_once("  ").unwrap();
            (name, digest)
        })
        .collect();
    assert_eq!(
        checksums.keys().copied().collect::<BTreeSet<_>>(),
        in_order.iter().copied().collect::<BTreeSet<_>>()
    );

    for (name, contents) in vectors::FILES {
        assert_eq!(
            format!("{:x}", Sha256::digest(contents.as_bytes())),
            checksums[name],
            "{name}: SHA-256 mismatch"
        );
    }
}
