//! Setup shared by the integration tests that build throwaway Cargo fixtures.

use std::sync::Once;

/// Keep the host's Cargo and nextest settings out of the fixtures.
///
/// Each test copies the same fixture package into a fresh temporary directory.
/// The copies have identical names and identical layouts relative to their
/// roots, so Cargo gives them identical artifact names and fingerprints. When
/// the host points every build at one shared target directory (a remote build
/// runner exporting `CARGO_TARGET_DIR`, or `build.target-dir` in a Cargo
/// config), tests running in parallel overwrite each other's fixture binaries.
/// Cargo decides freshness by comparing file modification times, so it can
/// then reuse another test's mutated binary as up to date for this test's
/// unmutated tree, and the clean-tree baseline goes red. A relative
/// `CARGO_TARGET_DIR` resolves against Cargo's working directory, and the
/// mutation runner always starts Cargo in the fixture root, so this one
/// process-wide value gives each fixture a private `<root>/target`. The
/// environment variable also overrides any `build.target-dir` configuration.
///
/// `NEXTEST_TEST_THREADS` takes precedence over a fixture's own
/// `.config/nextest.toml`. Fixtures that set `test-threads = 1` to fix the
/// order of their output would otherwise run in parallel whenever the host
/// exports that variable, and their output order would depend on machine load.
///
/// The library calls under test start Cargo with this process's environment,
/// so the change is made once, before a test spawns its first process.
pub fn isolate_fixture_environment() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::env::set_var("CARGO_TARGET_DIR", "target");
        std::env::remove_var("NEXTEST_TEST_THREADS");
    });
}
