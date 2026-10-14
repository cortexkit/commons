//! Portable helpers for CortexKit tests. Use this crate as a **dev-dependency only**.
//!
//! Scratch fixtures preserve failure evidence, executables are published as
//! immutable `ckdev-*` copies, and daemon builders enforce an ambient-root fence.
#![forbid(unsafe_code)]

#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::path::{Path, PathBuf};

mod binaries;
mod daemon;
mod fence;
mod scratch;
mod sibling_cache;
mod spawn_guard;
mod stable;

pub use binaries::*;
pub use daemon::{TestDaemon, TestDaemonCommand};
pub use fence::*;
pub use scratch::{
    process_alive, process_liveness, scratch_root, shared_scratch_root, wait_until_gone, Liveness,
    ScratchDir, STABLE_BIN_DIR,
};
pub use sibling_cache::*;
pub use spawn_guard::assert_test_binary_spawns;
#[cfg(unix)]
pub use stable::{stable_executable, stable_executable_dir};

/// Selects `TMPDIR` only when it is absolute and canonicalizes outside macOS's
/// `/var/folders` tree, where macOS can delay launches of newly copied executables;
/// otherwise it falls back to `/tmp`. Unix only: Windows keeps its system temp.
#[cfg(unix)]
pub(crate) fn private_temp_base(tmpdir: Option<&OsStr>) -> PathBuf {
    let Some(tmpdir) = tmpdir.filter(|value| !value.is_empty()) else {
        return PathBuf::from("/tmp");
    };
    let path = Path::new(tmpdir);
    let Ok(canonical_path) = std::fs::canonicalize(path) else {
        return PathBuf::from("/tmp");
    };
    if !path.is_absolute() || canonical_path.starts_with("/private/var/folders") {
        PathBuf::from("/tmp")
    } else {
        path.to_path_buf()
    }
}

/// Opt-in variable for tests requiring passwordless sudo, namespaces, or root.
pub const PRIVILEGED_TESTS_ENV: &str = "CORTEXKIT_PRIVILEGED_TESTS";

/// Returns true only for `CORTEXKIT_PRIVILEGED_TESTS=1`; otherwise prints a skip reason.
pub fn privileged_tests_enabled(test: &str, needs: &str) -> bool {
    if std::env::var(PRIVILEGED_TESTS_ENV).as_deref() == Ok("1") {
        return true;
    }
    eprintln!("skipped: {test} needs {needs}; set {PRIVILEGED_TESTS_ENV}=1 to run it");
    false
}

#[cfg(test)]
#[test]
fn privileged_gate_only_accepts_exact_one() {
    let previous = std::env::var_os(PRIVILEGED_TESTS_ENV);
    for value in [None, Some("0"), Some("true"), Some("1")] {
        match value {
            Some(value) => std::env::set_var(PRIVILEGED_TESTS_ENV, value),
            None => std::env::remove_var(PRIVILEGED_TESTS_ENV),
        }
        assert_eq!(
            privileged_tests_enabled("probe", "privilege"),
            value == Some("1")
        );
    }
    match previous {
        Some(value) => std::env::set_var(PRIVILEGED_TESTS_ENV, value),
        None => std::env::remove_var(PRIVILEGED_TESTS_ENV),
    }
}

#[cfg(all(test, unix))]
mod temp_base_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn test_directory(label: &str) -> PathBuf {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = crate_dir.join(format!(
            "cortexkit-test-support-{label}-{}-{nonce}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        path
    }

    #[test]
    fn private_temp_base_uses_non_var_tmpdir() {
        let tmpdir = test_directory("worker-tmpdir");
        assert_eq!(
            private_temp_base(Some(tmpdir.as_os_str())),
            tmpdir,
            "an absolute TMPDIR outside macOS's user-temp tree should be retained"
        );
        std::fs::remove_dir_all(tmpdir).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn private_temp_base_rejects_var_folders_even_through_var_symlink() {
        let private_root = Path::new("/private/var/folders");
        let var_alias = Path::new("/var/folders");
        assert!(private_root.is_dir());
        assert!(var_alias.is_dir());
        assert!(std::fs::canonicalize(var_alias)
            .unwrap()
            .starts_with(private_root));

        for tmpdir in [private_root, var_alias] {
            assert_eq!(
                private_temp_base(Some(tmpdir.as_os_str())),
                Path::new("/tmp"),
                "TMPDIR {} resolves into macOS's assessed user-temp tree",
                tmpdir.display()
            );
        }
    }

    #[test]
    fn private_temp_base_rejects_a_relative_tmpdir_that_exists() {
        // Cargo runs unit tests from the crate directory, so this relative path
        // resolves to a real directory; only the absolute-path rule can refuse it.
        let existing = test_directory("relative-tmpdir");
        let relative = existing
            .strip_prefix(env!("CARGO_MANIFEST_DIR"))
            .unwrap()
            .to_path_buf();
        assert!(relative.is_relative() && std::fs::canonicalize(&relative).is_ok());
        assert_eq!(
            private_temp_base(Some(relative.as_os_str())),
            Path::new("/tmp")
        );
        std::fs::remove_dir_all(existing).unwrap();
    }

    #[test]
    fn private_temp_base_rejects_unset_empty_and_relative_tmpdir() {
        assert_eq!(private_temp_base(None), Path::new("/tmp"));
        assert_eq!(private_temp_base(Some(OsStr::new(""))), Path::new("/tmp"));
        assert_eq!(
            private_temp_base(Some(OsStr::new("relative/tmp"))),
            Path::new("/tmp")
        );
    }
}
