#![cfg(windows)]

use cortexkit_lease::{
    create_private_dir, create_private_file, durable_replace, protect_dir_only, protect_file,
    reset_to_inherited, sync_dir, verify_private, NotPrivate,
};
use std::{io::Write, os::windows::ffi::OsStrExt, path::PathBuf};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn private_primitives_accept_paths_over_max_path() {
    let root = TempDir(std::env::temp_dir().join(format!(
        "cortexkit-lease-long-path-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    // A verbatim input would hide the regression in callers passing ordinary paths.
    assert!(!root.0.to_string_lossy().starts_with(r"\\?\"));
    let mut deep = root.0.clone();
    while deep.as_os_str().encode_wide().count() <= 320 {
        deep.push("long-directory-component");
    }
    std::fs::create_dir_all(&deep).unwrap();
    assert!(deep.as_os_str().encode_wide().count() > 300);

    let dir = deep.join("private-parent").join("private-dir");
    // Without verbatim conversion, CreateDirectoryW fails here with os error 206.
    create_private_dir(&dir).unwrap();
    verify_private(&dir).unwrap();
    let file = dir.join("private-file");
    drop(create_private_file(&file).unwrap());
    verify_private(&file).unwrap();

    reset_to_inherited(&file, false).unwrap();
    assert!(matches!(
        verify_private(&file),
        Err(NotPrivate::Unprotected)
    ));
    protect_file(&file).unwrap();
    verify_private(&file).unwrap();

    reset_to_inherited(&dir, false).unwrap();
    assert!(matches!(verify_private(&dir), Err(NotPrivate::Unprotected)));
    protect_dir_only(&dir).unwrap();
    verify_private(&dir).unwrap();
    verify_private(&file).unwrap();

    reset_to_inherited(&dir, true).unwrap();
    assert!(matches!(verify_private(&dir), Err(NotPrivate::Unprotected)));
    assert!(matches!(
        verify_private(&file),
        Err(NotPrivate::Unprotected)
    ));
    create_private_dir(&dir).unwrap();
    verify_private(&dir).unwrap();
    protect_file(&file).unwrap();
    verify_private(&file).unwrap();

    let temp = dir.join("replacement");
    let mut replacement = create_private_file(&temp).unwrap();
    replacement.write_all(b"replacement contents").unwrap();
    drop(replacement);
    durable_replace(&temp, &file).unwrap();
    assert!(!temp.exists());
    assert_eq!(std::fs::read(&file).unwrap(), b"replacement contents");
    verify_private(&file).unwrap();
    sync_dir(&dir).unwrap();

    let error = protect_file(&dir).unwrap_err().to_string();
    assert!(error.contains(&dir.display().to_string()));
    assert!(!error.contains(r"\\?\"));
}
