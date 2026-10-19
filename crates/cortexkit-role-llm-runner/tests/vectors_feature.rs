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
    let on_disk = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().file_name() != "SHA256SUMS")
        .count();
    assert_eq!(vectors::FILES.len(), on_disk);
    for (name, contents) in vectors::FILES {
        assert_eq!(vectors::file(name), Some(*contents), "{name}");
    }
    assert!(vectors::file("run_status.usage_plain.json").is_some());
    assert!(vectors::SHA256SUMS.lines().count() == vectors::FILES.len());
}
