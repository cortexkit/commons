use super::prepare_dir;
use cortexkit_lease::test_support as windows;
use std::fs;
use tempfile::TempDir;

#[test]
fn enforced_prepare_dir_removes_broad_inherited_acls_from_existing_segments() {
    let temp = TempDir::new().unwrap();
    windows::grant_everyone(temp.path());
    let parent_before = windows::read_acl(temp.path());
    let logs = temp.path().join("logs");
    fs::create_dir(&logs).unwrap();
    let segment = logs.join("insula.2026-09-04.log");
    fs::write(&segment, b"existing segment").unwrap();
    let nested = logs.join("archived");
    fs::create_dir(&nested).unwrap();
    let nested_segment = nested.join("insula.2026-09-03.log");
    fs::write(&nested_segment, b"existing nested segment").unwrap();
    for path in [&logs, &segment, &nested, &nested_segment] {
        windows::assert_broad_inherited(path);
    }

    let protected = logs.join("protected");
    fs::create_dir(&protected).unwrap();
    windows::grant_everyone(&protected);
    let protected_file = protected.join("custom.log");
    fs::write(&protected_file, b"custom security").unwrap();
    let protected_before = windows::read_acl(&protected);
    let protected_file_before = windows::read_acl(&protected_file);
    assert!(protected_before.protected);
    windows::assert_broad_inherited(&protected_file);

    // Call directory preparation alone: opening a segment would protect that
    // file separately and could hide a failure to propagate the directory ACL.
    prepare_dir(&logs, true).unwrap();

    windows::assert_owner_only(&logs, true, true);
    windows::assert_owner_only(&segment, false, false);
    windows::assert_owner_only(&nested, true, false);
    windows::assert_owner_only(&nested_segment, false, false);
    assert_eq!(windows::read_acl(temp.path()), parent_before);
    assert_eq!(windows::read_acl(&protected), protected_before);
    assert_eq!(windows::read_acl(&protected_file), protected_file_before);
}
