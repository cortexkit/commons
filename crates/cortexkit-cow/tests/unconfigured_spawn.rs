#![forbid(unsafe_code)]

mod common;

use cortexkit_cow::{backend, BackendKind};

#[cfg(target_os = "macos")]
#[tokio::test]
async fn unconfigured_macos_spawn_operations_fail_closed() {
    let root = tempfile::tempdir().unwrap();
    let lower = root.path().join("lower");
    let merged = root.path().join("merged");
    std::fs::create_dir(&lower).unwrap();
    common::seed_repo(&lower);
    let cow = backend(BackendKind::Rcopy);
    let error = cow.start(&lower, &merged).unwrap_err();
    assert!(
        error.message().contains("configure_spawn_trampoline"),
        "{error}"
    );
    assert!(
        !merged.exists(),
        "refused worktree spawn must not create a clone"
    );
    let error = cow.diff(&lower, &lower).await.unwrap_err();
    assert!(
        error.message().contains("configure_spawn_trampoline"),
        "{error}"
    );
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn non_macos_spawn_operations_need_no_trampoline() {
    let root = tempfile::tempdir().unwrap();
    let lower = root.path().join("lower");
    let merged = root.path().join("merged");
    std::fs::create_dir(&lower).unwrap();
    common::seed_repo(&lower);
    let cow = backend(BackendKind::Rcopy);
    cow.start(&lower, &merged).unwrap();
    assert!(cow.diff(&lower, &merged).await.unwrap().is_empty());
    cow.stop(&merged).unwrap();
}
