use super::test_support::{assert_broad_inherited, assert_owner_only, grant_everyone, read_acl};
use crate::{
    create_private_dir, create_private_file, durable_replace, protect_dir_only, protect_file,
    reset_to_inherited, sync_dir, verify_private, NotPrivate,
};
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

fn owner_sid(path: &std::path::Path) -> Vec<u8> {
    let (handle, _) = super::open_no_follow(path, super::READ_CONTROL).unwrap();
    let security = super::Security::read(&handle).unwrap();
    // SAFETY: GetSecurityInfo initialized the owner SID inside the live descriptor.
    unsafe {
        std::slice::from_raw_parts(
            security.owner.cast::<u8>(),
            super::GetLengthSid(security.owner) as usize,
        )
        .to_vec()
    }
}

fn junction(link: &std::path::Path, target: &std::path::Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink /J failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn security_descriptor(path: &std::path::Path) -> Vec<u8> {
    use windows_sys::Win32::Security::{GetSecurityDescriptorLength, GROUP_SECURITY_INFORMATION};
    let (handle, _) = super::open_no_follow(path, super::READ_CONTROL).unwrap();
    let mut raw = std::ptr::null_mut();
    // SAFETY: handle is live with READ_CONTROL, raw is writable, and the returned
    // self-relative descriptor is retained until its bytes have been copied.
    assert_eq!(
        unsafe {
            super::GetSecurityInfo(
                handle.0,
                super::SE_FILE_OBJECT,
                super::OWNER_SECURITY_INFORMATION
                    | GROUP_SECURITY_INFORMATION
                    | super::DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut raw,
            )
        },
        0
    );
    let descriptor = super::LocalAllocation(raw);
    // SAFETY: GetSecurityInfo returned a self-relative descriptor whose full
    // length is given by GetSecurityDescriptorLength. Copy before LocalFree.
    unsafe {
        std::slice::from_raw_parts(
            descriptor.0.cast::<u8>(),
            GetSecurityDescriptorLength(descriptor.0) as usize,
        )
        .to_vec()
    }
}

fn add_explicit_user_ace(path: &std::path::Path, mask: u32) {
    let user = super::User::current().unwrap();
    let (handle, _) = super::open_no_follow(path, super::READ_CONTROL | super::WRITE_DAC).unwrap();
    let mut acl = super::OwnerAcl(vec![0; 256]);
    // SAFETY: The aligned buffer has ample room for an ACL and one user ACE;
    // AddAccessAllowedAceEx copies the live token SID into the ACL.
    assert_ne!(
        unsafe { super::InitializeAcl(acl.as_mut_ptr(), 1024, super::ACL_REVISION) },
        0
    );
    // SAFETY: acl was initialized above and the user SID is live for this call.
    assert_ne!(
        unsafe {
            super::AddAccessAllowedAceEx(acl.as_mut_ptr(), super::ACL_REVISION, 0, mask, user.sid())
        },
        0
    );
    // SAFETY: The handle and ACL are live. The fixture deliberately keeps parent
    // inheritance enabled so a propagating permission writer would change it.
    assert_eq!(
        unsafe {
            super::SetSecurityInfo(
                handle.0,
                super::SE_FILE_OBJECT,
                super::DACL_SECURITY_INFORMATION | super::UNPROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl.as_mut_ptr(),
                std::ptr::null(),
            )
        },
        0
    );
    let snapshot = read_acl(path);
    assert!(!snapshot.protected);
    assert!(snapshot.aces.iter().any(|ace| ace.flags
        & windows_sys::Win32::Security::INHERITED_ACE as u8
        == 0
        && ace.mask == mask));
    assert_broad_inherited(path);
}

fn restrict_user_access_without_write_owner(path: &std::path::Path, directory: bool) {
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
    let (handle, _) = super::open_no_follow(path, super::READ_CONTROL | super::WRITE_DAC).unwrap();
    let mut acl = super::OwnerAcl::new(directory).unwrap();
    let mut raw = std::ptr::null_mut();
    // SAFETY: OwnerAcl initialized a live ACL with one ACCESS_ALLOWED ACE.
    assert_ne!(unsafe { super::GetAce(acl.as_mut_ptr(), 0, &mut raw) }, 0);
    let mask = FILE_GENERIC_READ | FILE_GENERIC_WRITE | super::READ_CONTROL | super::WRITE_DAC;
    // SAFETY: The returned ACE is the single ACCESS_ALLOWED ACE in OwnerAcl's
    // writable buffer. Only its access mask changes, not its SID or size.
    unsafe { (*raw.cast::<super::ACCESS_ALLOWED_ACE>()).Mask = mask };
    // SAFETY: handle has WRITE_DAC and the ACL is live. No owner change is
    // requested, so the file's explicitly assigned TokenUser owner is retained.
    assert_eq!(
        unsafe {
            super::SetSecurityInfo(
                handle.0,
                super::SE_FILE_OBJECT,
                super::DACL_SECURITY_INFORMATION | super::PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl.as_mut_ptr(),
                std::ptr::null(),
            )
        },
        0
    );
    assert_eq!(owner_sid(path), super::test_support::current_user_sid());
    let snapshot = read_acl(path);
    assert!(snapshot.protected);
    assert_eq!(snapshot.aces.len(), 1);
    assert_eq!(
        snapshot.aces[0].sid,
        super::test_support::current_user_sid()
    );
    assert_eq!(snapshot.aces[0].mask, mask);
    assert_eq!(mask & super::WRITE_OWNER, 0);
}

#[test]
fn named_security_helpers_accept_paths_over_max_path() {
    let temp = TempDir::new();
    let deep = temp
        .0
        .join("long-directory-component-".repeat(6))
        .join("long-directory-component-".repeat(6));
    assert!(deep.as_os_str().len() > 300);
    std::fs::create_dir_all(&deep).unwrap();
    grant_everyone(&deep);
    assert!(!read_acl(&deep).aces.is_empty());
    protect_dir_only(&deep).unwrap();
    assert_owner_only(&deep, true, true);
    verify_private(&deep).unwrap();
}

#[test]
fn protect_file_preserves_current_owner_without_requiring_write_owner() {
    let root = TempDir::new();
    let path = root.0.join("file");
    create_private_file(&path).unwrap();
    restrict_user_access_without_write_owner(&path, false);
    let protection = super::Protection::open(&path).unwrap();
    assert!(!protection.change_owner, "TokenUser already owns this file");
    assert_eq!(
        protection.information() & super::OWNER_SECURITY_INFORMATION,
        0
    );
    protect_file(&path).unwrap();
    verify_private(&path).unwrap();
    assert_owner_only(&path, false, true);
}

#[test]
fn directory_protection_preserves_current_owner_without_requiring_write_owner() {
    let root = TempDir::new();
    for (name, protect) in [
        (
            "create",
            create_private_dir as fn(&std::path::Path) -> std::io::Result<()>,
        ),
        ("only", protect_dir_only),
    ] {
        let path = root.0.join(name);
        create_private_dir(&path).unwrap();
        restrict_user_access_without_write_owner(&path, true);
        protect(&path).unwrap();
        verify_private(&path).unwrap();
        assert_owner_only(&path, true, true);
    }
}

#[test]
fn protect_dir_only_preserves_existing_child_security_descriptors_byte_for_byte() {
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_READ, FILE_GENERIC_WRITE};
    let root = TempDir::new();
    grant_everyone(&root.0);
    let dir = root.0.join("originals");
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join("file");
    let child_dir = dir.join("directory");
    std::fs::write(&file, b"original").unwrap();
    std::fs::create_dir(&child_dir).unwrap();
    add_explicit_user_ace(&file, FILE_GENERIC_READ);
    add_explicit_user_ace(&child_dir, FILE_GENERIC_WRITE);
    let file_before = security_descriptor(&file);
    let directory_before = security_descriptor(&child_dir);
    protect_dir_only(&dir).unwrap();
    verify_private(&dir).unwrap();
    assert_owner_only(&dir, true, true);
    assert_eq!(
        security_descriptor(&file),
        file_before,
        "existing file descriptor changed"
    );
    assert_eq!(
        security_descriptor(&child_dir),
        directory_before,
        "existing directory descriptor changed"
    );
    let new_child = dir.join("new");
    std::fs::write(&new_child, b"inherits").unwrap();
    assert_owner_only(&new_child, false, false);
}

#[test]
fn protect_dir_only_refuses_a_junction_and_leaves_the_target_unchanged() {
    let root = TempDir::new();
    let target = root.0.join("target");
    std::fs::create_dir(&target).unwrap();
    grant_everyone(&target);
    let before = security_descriptor(&target);
    let link = root.0.join("junction");
    junction(&link, &target);
    assert_eq!(
        protect_dir_only(&link).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(security_descriptor(&target), before);
    std::fs::remove_dir(&link).unwrap();
}

#[test]
fn verify_private_refuses_inherited_user_only_dacls_until_protected() {
    let root = TempDir::new();
    let parent = root.0.join("private");
    create_private_dir(&parent).unwrap();
    verify_private(&parent).unwrap();
    let file = parent.join("file");
    let directory = parent.join("directory");
    std::fs::write(&file, b"inherited").unwrap();
    std::fs::create_dir(&directory).unwrap();

    let user = super::User::current().unwrap();
    for (path, is_directory, protect) in [
        (
            &file,
            false,
            protect_file as fn(&std::path::Path) -> std::io::Result<()>,
        ),
        (&directory, true, protect_dir_only),
    ] {
        // An elevated token can default new entries to Administrators. Set only
        // the owner so the inherited, unprotected DACL remains the test subject.
        let (handle, _) = super::open_no_follow(path, super::WRITE_OWNER).unwrap();
        // SAFETY: The handle and user SID are live; only the owner is changed.
        assert_eq!(
            unsafe {
                super::SetSecurityInfo(
                    handle.0,
                    super::SE_FILE_OBJECT,
                    super::OWNER_SECURITY_INFORMATION,
                    user.sid(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                )
            },
            0
        );
        assert_eq!(owner_sid(path), super::test_support::current_user_sid());
        assert_owner_only(path, is_directory, false);
        let error = verify_private(path).unwrap_err();
        assert!(matches!(error, NotPrivate::Unprotected), "{error}");

        protect(path).unwrap();
        verify_private(path).unwrap();
        assert_owner_only(path, is_directory, true);
    }
}

#[test]
fn verify_private_refuses_other_principal_allow_ace_and_names_its_sid() {
    let root = TempDir::new();
    let path = root.0.join("file");
    create_private_file(&path).unwrap();
    grant_everyone(&path);
    let error = verify_private(&path).unwrap_err();
    assert!(
        matches!(&error, NotPrivate::Access { principal } if principal.starts_with("S-1-1-0")),
        "{error}"
    );
}

#[test]
fn verify_private_refuses_a_null_dacl_and_names_everyone() {
    let root = TempDir::new();
    let path = root.0.join("file");
    create_private_file(&path).unwrap();
    let (handle, _) = super::open_no_follow(&path, super::READ_CONTROL | super::WRITE_DAC).unwrap();
    // SAFETY: handle is live; a null DACL intentionally grants everyone access
    // in this disposable fixture. No owner/group/SACL changes are requested.
    assert_eq!(
        unsafe {
            super::SetSecurityInfo(
                handle.0,
                super::SE_FILE_OBJECT,
                super::DACL_SECURITY_INFORMATION | super::PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        },
        0
    );
    let error = verify_private(&path).unwrap_err();
    assert!(
        matches!(&error, NotPrivate::Access { principal } if principal.starts_with("S-1-1-0")),
        "{error}"
    );
}

#[test]
fn protect_file_sets_current_user_owner_even_with_an_elevated_default_owner() {
    let root = TempDir::new();
    let path = root.0.join("file");
    std::fs::write(&path, b"private").unwrap();
    let expected = super::test_support::current_user_sid();
    let default_owner = owner_sid(&path);
    if default_owner != expected {
        assert!(
            matches!(verify_private(&path), Err(NotPrivate::Owner { principal }) if principal.starts_with("S-1-"))
        );
    } else {
        eprintln!("default owner is already TokenUser; elevated-owner branch is not exercised on this token");
    }
    protect_file(&path).unwrap();
    assert_eq!(owner_sid(&path), expected);
    verify_private(&path).unwrap();
}

#[test]
fn create_private_file_has_a_protected_owner_only_dacl_and_current_user_owner() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let path = root.0.join("file");
    create_private_file(&path).unwrap();
    assert_owner_only(&path, false, true);
    assert_eq!(owner_sid(&path), super::test_support::current_user_sid());
    verify_private(&path).unwrap();
}

#[test]
fn verify_private_refuses_a_junction_even_to_a_private_target() {
    let root = TempDir::new();
    let target = root.0.join("target");
    create_private_dir(&target).unwrap();
    let link = root.0.join("junction");
    junction(&link, &target);
    assert!(matches!(verify_private(&link), Err(NotPrivate::Link)));
    std::fs::remove_dir(&link).unwrap();
}

#[test]
fn reset_to_inherited_restores_parent_aces_recursively_without_changing_owners() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let dir = root.0.join("private");
    let nested = dir.join("nested");
    create_private_dir(&nested).unwrap();
    let direct = dir.join("file");
    let file = nested.join("file");
    create_private_file(&direct).unwrap();
    create_private_file(&file).unwrap();
    let paths = [&dir, &nested, &direct, &file];
    let owners: Vec<_> = paths.iter().map(|path| owner_sid(path)).collect();
    for path in paths {
        verify_private(path).unwrap();
    }
    reset_to_inherited(&dir, true).unwrap();
    for (path, owner) in paths.into_iter().zip(owners) {
        assert_broad_inherited(path);
        let acl = read_acl(path);
        assert!(
            acl.aces
                .iter()
                .all(|ace| ace.flags & windows_sys::Win32::Security::INHERITED_ACE as u8 != 0),
            "no explicit ACE should remain: {acl:?}"
        );
        assert_eq!(owner_sid(path), owner);
    }
}

#[test]
fn reset_to_inherited_is_nonrecursive_when_requested_and_missing_paths_error() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let dir = root.0.join("private");
    create_private_dir(&dir).unwrap();
    let file = dir.join("file");
    create_private_file(&file).unwrap();
    let before = read_acl(&file);
    reset_to_inherited(&dir, false).unwrap();
    assert_broad_inherited(&dir);
    assert_eq!(
        read_acl(&file),
        before,
        "protected child must not be reset nonrecursively"
    );
    reset_to_inherited(&file, false).unwrap();
    assert_broad_inherited(&file);
    assert_eq!(
        reset_to_inherited(&root.0.join("missing"), true)
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::NotFound
    );
}

#[test]
fn recursive_reset_does_not_follow_a_junction_outside_the_tree() {
    let root = TempDir::new();
    grant_everyone(&root.0);
    let tree = root.0.join("tree");
    let outside = root.0.join("outside");
    create_private_dir(&tree).unwrap();
    create_private_dir(&outside).unwrap();
    let outside_child = outside.join("child");
    create_private_dir(&outside_child).unwrap();
    let outside_file = outside_child.join("file");
    create_private_file(&outside_file).unwrap();
    let before = read_acl(&outside);
    let child_before = read_acl(&outside_child);
    let file_before = read_acl(&outside_file);
    let link = tree.join("junction");
    junction(&link, &outside);
    reset_to_inherited(&tree, true).unwrap();
    assert_eq!(
        read_acl(&outside),
        before,
        "junction target must be unchanged"
    );
    assert_eq!(
        read_acl(&outside_child),
        child_before,
        "walk must not leave the tree"
    );
    assert_eq!(read_acl(&outside_file), file_before);
    let (handle, attributes) = super::open_no_follow(&link, super::READ_CONTROL).unwrap();
    assert_ne!(attributes & super::FILE_ATTRIBUTE_REPARSE_POINT, 0);
    let security = super::Security::read(&handle).unwrap();
    let mut control = 0;
    let mut revision = 0;
    // SAFETY: The descriptor is live and both control outputs are writable.
    assert_ne!(
        unsafe {
            windows_sys::Win32::Security::GetSecurityDescriptorControl(
                security._descriptor.0,
                &mut control,
                &mut revision,
            )
        },
        0
    );
    assert_eq!(
        control & super::SE_DACL_PROTECTED,
        0,
        "junction itself must be reset"
    );
    drop(handle);
    std::fs::remove_dir(&link).unwrap();
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
