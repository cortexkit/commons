//! Portable helpers for CortexKit tests. Use this crate as a **dev-dependency only**.
//!
//! Scratch fixtures preserve failure evidence, executables are published as
//! immutable `ckdev-*` copies, and daemon builders enforce an ambient-root fence.
#![forbid(unsafe_code)]

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
pub use scratch::{scratch_root, shared_scratch_root, wait_until_gone, ScratchDir, STABLE_BIN_DIR};
pub use sibling_cache::*;
pub use spawn_guard::assert_test_binary_spawns;
#[cfg(unix)]
pub use stable::{stable_executable, stable_executable_dir};

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
