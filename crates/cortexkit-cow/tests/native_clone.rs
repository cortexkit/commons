#![forbid(unsafe_code)]

#[cfg(any(target_os = "macos", target_os = "linux"))]
use cortexkit_cow::{backend, BackendKind};

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn native_clone_isolates_writes_without_copy_fallback() {
    let root = tempfile::tempdir().unwrap();
    let lower = root.path().join("lower");
    let merged = root.path().join("merged");
    std::fs::create_dir(&lower).unwrap();
    std::fs::write(lower.join("file.txt"), "before\n").unwrap();
    let cow = backend(BackendKind::native());
    match cow.start(&lower, &merged) {
        Ok(()) => {
            assert_eq!(std::fs::read(merged.join("file.txt")).unwrap(), b"before\n");
            std::fs::write(merged.join("file.txt"), "after\n").unwrap();
            assert_eq!(std::fs::read(lower.join("file.txt")).unwrap(), b"before\n");
            cow.stop(&merged).unwrap();
            eprintln!("native clone ran successfully in {}", root.path().display());
        }
        Err(error) => {
            assert!(error.is_unavailable(), "{error}");
            assert!(
                std::env::var_os("CORTEXKIT_COW_REQUIRE_NATIVE_CLONE").is_none(),
                "native clone required: {error}"
            );
            eprintln!("native clone unavailable: {error}");
        }
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_reflink_cross_device_reports_unavailable_without_copying_bytes() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir_in("/dev/shm").unwrap();
    let lower = root.path().join("lower");
    let merged = other.path().join("merged");
    std::fs::create_dir(&lower).unwrap();
    std::fs::write(lower.join("file.txt"), "must not be copied\n").unwrap();
    let cow = backend(BackendKind::LinuxReflink);
    let error = cow.start(&lower, &merged).unwrap_err();
    assert!(error.is_unavailable(), "{error}");
    assert!(!merged.join("file.txt").exists());
}
