#![forbid(unsafe_code)]

use std::path::Path;

pub fn git(cwd: &Path, args: &[&str]) {
    // Fixture setup runs git directly, without the library's process-wide
    // trampoline, so a test that never configures one can still create a real
    // repository.
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

pub fn seed_repo(dir: &Path) {
    git(dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("file.txt"), "before\n").unwrap();
    git(dir, &["add", "file.txt"]);
    git(
        dir,
        &[
            "-c",
            "user.email=cow@test.invalid",
            "-c",
            "user.name=cow",
            "commit",
            "-q",
            "-m",
            "seed",
        ],
    );
}
