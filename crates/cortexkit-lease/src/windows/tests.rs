use super::test_support::{assert_broad_inherited, assert_owner_only, grant_everyone, read_acl};
use crate::{create_private_dir, durable_replace, protect_file, sync_dir};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "cortexkit-windows-acl-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn create_private_dir_has_a_protected_user_only_dacl_and_private_children() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let dir = root.0.join("parent").join("owned");
    create_private_dir(&dir).unwrap();
    assert_owner_only(&root.0.join("parent"), true, true);
    assert_owner_only(&dir, true, true);
    let child = dir.join("inherited.txt");
    std::fs::write(&child, b"private").unwrap();
    // Inheritance alone leaves the child's DACL unprotected, but carries only
    // the parent's one user ACE. Explicit protection is tested separately.
    assert_owner_only(&child, false, false);
    let child_dir = dir.join("inherited-dir");
    std::fs::create_dir(&child_dir).unwrap();
    assert_owner_only(&child_dir, true, false);
}

#[test]
fn protect_file_replaces_an_inherited_broad_acl_with_a_protected_user_only_dacl() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let path = root.0.join("inherited.txt");
    std::fs::write(&path, b"private").unwrap();
    assert_broad_inherited(&path);
    protect_file(&path).unwrap();
    assert_owner_only(&path, false, true);
}

#[test]
fn create_private_dir_narrows_an_existing_broad_directory_without_changing_its_parent() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let before = read_acl(&root.0);
    let dir = root.0.join("existing");
    std::fs::create_dir(&dir).unwrap();
    assert_broad_inherited(&dir);
    create_private_dir(&dir).unwrap();
    assert_owner_only(&dir, true, true);
    assert_eq!(
        read_acl(&root.0),
        before,
        "the existing parent must not be narrowed"
    );
}

#[test]
fn create_private_dir_removes_broad_inherited_acls_from_existing_descendants() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let parent_before = read_acl(&root.0);
    let dir = root.0.join("existing");
    std::fs::create_dir(&dir).unwrap();
    let direct = dir.join("store.db");
    std::fs::write(&direct, b"existing store").unwrap();
    let nested = dir.join("logs");
    std::fs::create_dir(&nested).unwrap();
    let nested_file = nested.join("existing.log");
    std::fs::write(&nested_file, b"existing log").unwrap();
    for path in [&dir, &direct, &nested, &nested_file] {
        assert_broad_inherited(path);
    }

    let protected = dir.join("protected");
    std::fs::create_dir(&protected).unwrap();
    grant_everyone(&protected);
    let protected_file = protected.join("custom.txt");
    std::fs::write(&protected_file, b"custom security").unwrap();
    let protected_before = read_acl(&protected);
    let protected_file_before = read_acl(&protected_file);
    assert!(protected_before.protected);
    assert_broad_inherited(&protected_file);

    create_private_dir(&dir).unwrap();

    assert_owner_only(&dir, true, true);
    // Windows can bypass directory traversal checks, so the existing files'
    // own inherited ACLs must lose the broad entry when the parent is narrowed.
    assert_owner_only(&direct, false, false);
    assert_owner_only(&nested, true, false);
    assert_owner_only(&nested_file, false, false);
    assert_eq!(read_acl(&root.0), parent_before);
    assert_eq!(read_acl(&protected), protected_before);
    assert_eq!(read_acl(&protected_file), protected_file_before);
}

#[test]
fn protect_file_refuses_non_regular_files_and_directory_reparse_points_are_unchanged() {
    let root = TempDir::new();
    let target = root.0.join("target");
    std::fs::create_dir(&target).unwrap();
    grant_everyone(&target);
    let before = read_acl(&target);
    assert_eq!(
        protect_file(&target).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    let link = root.0.join("junction");
    // A junction is a directory reparse point and needs no symlink privilege,
    // so this refusal check runs even on Windows hosts without Developer Mode.
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        protect_file(&link).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    create_private_dir(&link).unwrap();
    assert_eq!(
        read_acl(&target),
        before,
        "the reparse target's DACL must be untouched"
    );
    std::fs::remove_dir(&link).unwrap();
}

#[test]
fn protect_file_refuses_file_symlinks_when_the_host_allows_them() {
    let root = TempDir::new();
    let target = root.0.join("target.txt");
    std::fs::write(&target, b"unchanged").unwrap();
    grant_everyone(&target);
    let before = read_acl(&target);
    let link = root.0.join("link.txt");
    if let Err(error) = std::os::windows::fs::symlink_file(&target, &link) {
        if error.raw_os_error() == Some(1314) {
            eprintln!(
                "file symlink requires a privilege; junction refusal is tested unconditionally"
            );
            return;
        }
        panic!("symlink_file: {error}");
    }
    assert_eq!(
        protect_file(&link).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(
        read_acl(&target),
        before,
        "the symlink target's DACL must be untouched"
    );
}

#[cfg(windows)]
#[test]
fn durable_replace_replaces_contents_and_removes_temp_file() {
    let root = TempDir::new();
    let temp = root.0.join("state.tmp");
    let dest = root.0.join("state");
    std::fs::write(&temp, b"new contents").expect("write temporary file");
    std::fs::write(&dest, b"old contents").expect("write existing file");

    durable_replace(&temp, &dest).expect("replace durably");

    assert_eq!(
        std::fs::read(&dest).expect("read destination"),
        b"new contents"
    );
    assert!(!temp.exists(), "the temporary name must be removed");
}

#[cfg(windows)]
#[test]
fn durable_replace_refuses_different_directories() {
    let root = TempDir::new();
    let temp_dir = root.0.join("temp");
    let dest_dir = root.0.join("destination");
    std::fs::create_dir(&temp_dir).expect("create temporary directory");
    std::fs::create_dir(&dest_dir).expect("create destination directory");
    let temp = temp_dir.join("state.tmp");
    let dest = dest_dir.join("state");
    std::fs::write(&temp, b"new contents").expect("write temporary file");
    std::fs::write(&dest, b"old contents").expect("write existing file");

    let error = durable_replace(&temp, &dest).expect_err("different directories must fail");

    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(
        std::fs::read(&temp).expect("read temporary file"),
        b"new contents"
    );
    assert_eq!(
        std::fs::read(&dest).expect("read destination"),
        b"old contents"
    );
}

#[cfg(windows)]
#[test]
fn sync_dir_succeeds_on_a_real_directory() {
    let root = TempDir::new();

    sync_dir(&root.0).expect("sync directory");
}
