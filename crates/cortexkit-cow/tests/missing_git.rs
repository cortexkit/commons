#![forbid(unsafe_code)]

use cortexkit_cow::{backend, BackendKind};

#[tokio::test]
async fn missing_git_cli_remains_unavailable() {
    let root = tempfile::tempdir().unwrap();
    if std::env::var_os("CORTEXKIT_COW_TEST_EMPTY_PATH").is_none() {
        // Set PATH only in a fresh test process, never in this multi-threaded
        // test runtime. Both platforms must classify a missing CLI identically.
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "missing_git_cli_remains_unavailable",
                "--nocapture",
            ])
            .env("CORTEXKIT_COW_TEST_EMPTY_PATH", "1")
            .env("PATH", root.path())
            .status()
            .unwrap();
        assert!(status.success());
        return;
    }
    cortexkit_cow::configure_spawn_trampoline(env!("CARGO_BIN_EXE_cortexkit-cow-spawn-fixture"))
        .unwrap();
    let lower = root.path().join("lower");
    let merged = root.path().join("merged");
    std::fs::create_dir_all(lower.join(".git")).unwrap();
    let cow = backend(BackendKind::Rcopy);
    assert!(cow.start(&lower, &merged).unwrap_err().is_unavailable());
    assert!(cow.diff(&lower, &lower).await.unwrap_err().is_unavailable());
}
