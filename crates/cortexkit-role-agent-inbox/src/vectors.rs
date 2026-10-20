//! Packaged `agent-inbox/v1` vectors, compiled in for tests or `test-vectors`.

pub const DIR: &str = "agent-inbox-v1";
pub const FILES: &[(&str, &str)] = &[
    (
        "neutralize.json",
        include_str!("../test-vectors/agent-inbox-v1/neutralize.json"),
    ),
    (
        "posts.json",
        include_str!("../test-vectors/agent-inbox-v1/posts.json"),
    ),
    (
        "pull.json",
        include_str!("../test-vectors/agent-inbox-v1/pull.json"),
    ),
    (
        "refusals.json",
        include_str!("../test-vectors/agent-inbox-v1/refusals.json"),
    ),
    (
        "withdraw.json",
        include_str!("../test-vectors/agent-inbox-v1/withdraw.json"),
    ),
    (
        "README.md",
        include_str!("../test-vectors/agent-inbox-v1/README.md"),
    ),
];
pub const SHA256SUMS: &str = include_str!("../test-vectors/agent-inbox-v1/SHA256SUMS");
pub const README: &str = include_str!("../test-vectors/agent-inbox-v1/README.md");

pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    #[test]
    fn every_vector_file_on_disk_is_listed_with_its_bytes() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("test-vectors")
            .join(DIR);
        let mut on_disk = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_str().unwrap().to_owned();
            if name != "SHA256SUMS" {
                on_disk.push(name);
            }
        }
        on_disk.sort();
        let mut listed: Vec<_> = FILES.iter().map(|(name, _)| name.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
        for (name, contents) in FILES {
            assert_eq!(
                std::fs::read_to_string(dir.join(name)).unwrap(),
                *contents,
                "{name}"
            );
            assert_eq!(file(name), Some(*contents));
        }
        assert_eq!(file("missing"), None);
        assert_eq!(file("README.md"), Some(README));
        assert_eq!(
            std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap(),
            SHA256SUMS
        );
    }

    #[test]
    fn files_match_sha256sums() {
        let lines: Vec<_> = SHA256SUMS.lines().collect();
        assert_eq!(lines.len(), FILES.len());
        for (line, (name, bytes)) in lines.iter().zip(FILES) {
            let (hash, file) = line.split_once("  ").unwrap();
            assert_eq!(file, *name);
            assert_eq!(
                hash,
                format!("{:x}", Sha256::digest(bytes.as_bytes())),
                "{name}"
            );
        }
    }
}
