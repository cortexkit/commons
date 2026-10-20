//! With the `test-vectors` feature, the `vectors` module is public and
//! carries the bytes of every file in `test-vectors/llm-runner-v1/`.

use cortexkit_role_llm_runner::vectors;

#[test]
fn the_public_module_carries_every_file() {
    let dir = format!(
        "{}/test-vectors/{}",
        env!("CARGO_MANIFEST_DIR"),
        vectors::DIR
    );
    fn count_files(path: &std::path::Path) -> usize {
        std::fs::read_dir(path)
            .unwrap()
            .map(|entry| {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    count_files(&entry.path())
                } else {
                    usize::from(entry.file_name() != "SHA256SUMS")
                }
            })
            .sum()
    }
    let on_disk = count_files(std::path::Path::new(&dir));
    assert_eq!(vectors::FILES.len(), on_disk);
    for (name, contents) in vectors::FILES {
        assert_eq!(vectors::file(name), Some(*contents), "{name}");
    }
    assert!(vectors::file("run_status.usage_plain.json").is_some());
    assert!(vectors::SHA256SUMS.lines().count() == vectors::FILES.len());
}
