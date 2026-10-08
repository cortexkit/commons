#![forbid(unsafe_code)]

mod common;

use cortexkit_cow::{backend, configure_spawn_trampoline, BackendKind, ChangeKind};
use std::path::PathBuf;

// Each integration test file is its own executable, so the trampoline this file
// configures (a process-wide OnceLock) can't leak into the unconfigured tests.
#[tokio::test]
async fn configured_trampoline_runs_worktree_dirty_seeding_and_async_diff() {
    configure_for_tests();
    #[cfg(target_os = "macos")]
    assert_eq!(
        configure_spawn_trampoline(env!("CARGO_BIN_EXE_cortexkit-cow-spawn-fixture"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::AlreadyExists
    );

    let root = tempfile::tempdir().unwrap();
    let lower = root.path().join("lower");
    let merged = root.path().join("merged");
    std::fs::create_dir(&lower).unwrap();
    common::seed_repo(&lower);
    std::fs::write(lower.join("file.txt"), "staged\n").unwrap();
    common::git(&lower, &["add", "file.txt"]);
    std::fs::write(lower.join("file.txt"), "unstaged\n").unwrap();
    std::fs::write(lower.join("new.txt"), "fresh\n").unwrap();

    let cow = backend(BackendKind::Rcopy);
    cow.start(&lower, &merged).unwrap();
    assert_eq!(
        std::fs::read(merged.join("file.txt")).unwrap(),
        b"unstaged\n"
    );
    assert_eq!(std::fs::read(merged.join("new.txt")).unwrap(), b"fresh\n");
    let diff = cow.diff(&lower, &merged).await.unwrap();
    assert_eq!(diff.files.len(), 2);
    assert_eq!(diff.files[0].path, PathBuf::from("file.txt"));
    assert_eq!(diff.files[0].op, ChangeKind::Modified);
    assert_eq!(diff.files[1].path, PathBuf::from("new.txt"));
    assert_eq!(diff.files[1].op, ChangeKind::Added);
    cow.stop(&merged).unwrap();
    assert!(!merged.exists());
}

// A repository with diff.noprefix=true or diff.external configured used to
// erase the modification. The command must pin the patch format itself.
#[tokio::test]
async fn a_repository_with_diff_noprefix_still_reports_its_changes() {
    // The other configured-spawn test may be running concurrently; initialize
    // configuration once for both tests without treating an existing value as
    // proof that initialization succeeded.
    configure_for_tests();
    let root = tempfile::tempdir().unwrap();
    let dir = root.path();
    common::seed_repo(dir);
    common::git(dir, &["config", "diff.noprefix", "true"]);
    common::git(
        dir,
        &["config", "diff.external", "printf external-diff-output"],
    );
    std::fs::write(dir.join("file.txt"), "after\n").unwrap();
    std::fs::write(dir.join("new.txt"), "fresh\n").unwrap();
    let diff = backend(BackendKind::Rcopy).diff(dir, dir).await.unwrap();
    let paths: Vec<_> = diff.files.iter().map(|file| file.path.clone()).collect();
    assert_eq!(
        paths,
        vec![PathBuf::from("file.txt"), PathBuf::from("new.txt")],
        "the tracked modification and the untracked file both survive diff.noprefix"
    );
}

fn configure_for_tests() {
    static CONFIGURED: std::sync::Once = std::sync::Once::new();
    CONFIGURED.call_once(|| {
        configure_spawn_trampoline(env!("CARGO_BIN_EXE_cortexkit-cow-spawn-fixture")).unwrap();
    });
}
