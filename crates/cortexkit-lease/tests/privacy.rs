#![cfg(any(unix, windows))]

use cortexkit_lease::{
    create_private_dir, create_private_file, protect_dir_only, protect_file, verify_private,
};
use std::{
    io::{Read, Seek, SeekFrom, Write},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "cortexkit-lease-privacy-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn verify_private_accepts_directory_and_protected_file_round_trip() {
    let root = TempDir::new();
    let dir = root.0.join("parent").join("private");
    create_private_dir(&dir).unwrap();
    verify_private(&dir).unwrap();
    verify_private(dir.parent().unwrap()).unwrap();
    let path = dir.join("file");
    std::fs::write(&path, b"private").unwrap();
    protect_file(&path).unwrap();
    verify_private(&path).unwrap();
}

#[test]
fn create_private_file_is_private_read_write_and_exclusive() {
    let root = TempDir::new();
    let path = root.0.join("new");
    let mut file = create_private_file(&path).unwrap();
    verify_private(&path).unwrap();
    file.write_all(b"private").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents = String::new();
    file.read_to_string(&mut contents).unwrap();
    assert_eq!(contents, "private");
    assert_eq!(
        create_private_file(&path).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
}

#[test]
fn verify_private_refuses_a_missing_path() {
    let root = TempDir::new();
    assert!(
        matches!(verify_private(&root.0.join("missing")), Err(cortexkit_lease::NotPrivate::Io(error)) if error.kind() == std::io::ErrorKind::NotFound)
    );
}

#[test]
fn protect_dir_only_makes_an_existing_directory_private_and_refuses_files() {
    let root = TempDir::new();
    let dir = root.0.join("directory");
    std::fs::create_dir(&dir).unwrap();
    protect_dir_only(&dir).unwrap();
    verify_private(&dir).unwrap();
    let file = dir.join("file");
    create_private_file(&file).unwrap();
    assert!(protect_dir_only(&file).is_err());
    verify_private(&file).unwrap();
    assert_eq!(
        protect_dir_only(&root.0.join("missing"))
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
}

#[cfg(unix)]
#[test]
fn protect_dir_only_preserves_child_modes_and_refuses_symlinks() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new();
    let dir = root.0.join("directory");
    std::fs::create_dir(&dir).unwrap();
    let child = dir.join("file");
    std::fs::write(&child, b"original").unwrap();
    std::fs::set_permissions(&child, std::fs::Permissions::from_mode(0o644)).unwrap();
    protect_dir_only(&dir).unwrap();
    verify_private(&dir).unwrap();
    assert_eq!(
        std::fs::metadata(&child).unwrap().permissions().mode() & 0o777,
        0o644
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    let link = root.0.join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    assert!(protect_dir_only(&link).is_err());
    assert_eq!(
        std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
        0o755
    );
}

#[cfg(unix)]
#[test]
fn verify_private_names_the_group_or_everyone_with_access() {
    use cortexkit_lease::NotPrivate;
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let root = TempDir::new();
    let path = root.0.join("file");
    std::fs::write(&path, b"public").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
    let gid = std::fs::metadata(&path).unwrap().gid();
    let error = verify_private(&path).unwrap_err();
    assert!(
        matches!(&error, NotPrivate::Access { principal } if principal == &format!("group (gid {gid})")),
        "{error}"
    );
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o604)).unwrap();
    let error = verify_private(&path).unwrap_err();
    assert!(
        matches!(&error, NotPrivate::Access { principal } if principal == "everyone (other)"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn verify_and_create_private_file_refuse_symlinks() {
    let root = TempDir::new();
    let target = root.0.join("target");
    create_private_file(&target).unwrap();
    let link = root.0.join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(matches!(
        verify_private(&link),
        Err(cortexkit_lease::NotPrivate::Link)
    ));
    assert_eq!(
        create_private_file(&link).unwrap_err().kind(),
        std::io::ErrorKind::AlreadyExists
    );
    verify_private(&target).unwrap();
}

#[cfg(unix)]
#[test]
fn reset_to_inherited_is_a_noop_and_missing_paths_error() {
    use cortexkit_lease::reset_to_inherited;
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new();
    let dir = root.0.join("dir");
    create_private_dir(&dir).unwrap();
    let file = dir.join("file");
    create_private_file(&file).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    for recursive in [false, true] {
        reset_to_inherited(&file, recursive).unwrap();
        reset_to_inherited(&dir, recursive).unwrap();
        assert_eq!(
            std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            reset_to_inherited(&root.0.join("missing"), recursive)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::NotFound
        );
    }
}
