//! `test-vectors/SHA256SUMS` pins every vector file's exact bytes, so an
//! accidental edit (or a line-ending conversion on checkout) fails here by
//! file name instead of surfacing as a confusing golden mismatch.

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

const ROOT: &str = "test-vectors";

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn sha256sums_lists_every_vector_file_with_its_exact_digest() {
    let sums = fs::read_to_string(format!("{ROOT}/SHA256SUMS")).unwrap();
    let mut listed = BTreeSet::new();
    for line in sums.lines() {
        let (digest, path) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("malformed line {line:?}"));
        let bytes = fs::read(format!("{ROOT}/{path}"))
            .unwrap_or_else(|e| panic!("{path} is listed but cannot be read: {e}"));
        assert_eq!(
            hex(&Sha256::digest(&bytes)),
            digest,
            "{path} does not match SHA256SUMS"
        );
        assert!(listed.insert(path.to_string()), "{path} is listed twice");
    }
    let mut files = Vec::new();
    walk(Path::new(ROOT), &mut files);
    let on_disk: BTreeSet<String> = files
        .iter()
        .map(|p| {
            p.strip_prefix(ROOT)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_str().unwrap())
                .collect::<Vec<_>>()
                .join("/")
        })
        .filter(|p| p != "SHA256SUMS")
        .collect();
    assert_eq!(on_disk, listed, "files on disk and in SHA256SUMS differ");
    // Vacuity floor: the shipped vectors are far more than this.
    assert!(listed.len() >= 25, "only {} files listed", listed.len());
}
