//! Finding the Cargo profile folder a test was built into.
//!
//! Tests that run a workspace `[[bin]]` (a stub module, a fixture helper, the
//! daemon itself) look for it in the profile folder, `target/debug` or
//! `target/release`, where `cargo build` and `cargo test` place binaries.
//! Cargo up to 1.99 built test executables in `<profile>/deps/`, so tests
//! reached the profile folder by going two folders up from their own path.
//! Cargo 1.100 builds them in `<profile>/build/<package>/<hash>/out/`, and the
//! same two steps now land inside the build folder. These functions work for
//! both layouts, and for a build folder kept apart from the target folder.

use std::path::{Path, PathBuf};

/// The profile folder for a test executable at `exe`: the parent of the
/// nearest enclosing `deps` or `build` folder. `None` when `exe` is in
/// neither, as when it was copied out of the target folder.
///
/// The binaries are only there when Cargo's build folder is its target folder,
/// which is the default. A separate `build.build-dir` keeps the test
/// executables apart from the binaries, and no path from the executable leads
/// to them; [`cargo_profile_dir`] handles that case from the environment.
pub fn cargo_profile_dir_of(exe: &Path) -> Option<PathBuf> {
    exe.ancestors()
        .skip(1)
        .find(|dir| {
            matches!(
                dir.file_name().and_then(|n| n.to_str()),
                Some("deps" | "build")
            )
        })
        .and_then(Path::parent)
        .map(Path::to_path_buf)
}

/// The profile folder holding the workspace's binaries for the running test.
///
/// Starts from [`cargo_profile_dir_of`] the test executable. When a separate
/// Cargo build folder is in use (`CARGO_BUILD_BUILD_DIR` or `build.build-dir`),
/// that path is a profile folder inside the build folder, and the binaries are
/// in the target folder's profile of the same name instead. The target folder
/// is read from `CARGO_TARGET_DIR` or `CARGO_BUILD_TARGET_DIR`, which the test
/// process inherits from the environment `cargo test` ran in; when it names an
/// existing profile folder of that name, that folder is returned.
///
/// Panics when the executable's path can't be read or isn't inside a profile
/// folder, because every caller would otherwise look for its binaries in the
/// wrong place and fail with a less helpful message.
pub fn cargo_profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test executable's path is readable");
    let built = cargo_profile_dir_of(&exe).unwrap_or_else(|| {
        panic!(
            "test executable {} is not inside a Cargo profile's deps/ or build/ folder",
            exe.display()
        )
    });
    target_profile_dir(&built, target_dir_from_env().as_deref()).unwrap_or(built)
}

fn target_dir_from_env() -> Option<PathBuf> {
    ["CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The target folder's profile folder named like `built`, when `target` is
/// given and that folder exists. `None` keeps `built`.
fn target_profile_dir(built: &Path, target: Option<&Path>) -> Option<PathBuf> {
    let profile = built.file_name()?;
    let candidate = target?.join(profile);
    candidate.is_dir().then_some(candidate)
}

/// A workspace binary named `name` in the profile folder [`cargo_profile_dir`]
/// returns, with the platform's executable suffix (`.exe` on Windows). The
/// file may not exist; callers check, so they can say which build step makes
/// it.
pub fn cargo_profile_bin(name: &str) -> PathBuf {
    cargo_profile_dir().join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_old_layout_resolves_two_folders_up_from_deps() {
        let exe = Path::new("/w/target/debug/deps/subc_jsonc-0ce8c6bd5f3f9830");
        assert_eq!(
            cargo_profile_dir_of(exe),
            Some(PathBuf::from("/w/target/debug"))
        );
    }

    #[test]
    fn the_cargo_1_100_layout_resolves_above_build() {
        let exe = Path::new(
            "/w/target/release/build/subc-jsonc/33fd7aabacfa86e6/out/subc_jsonc-33fd7aabacfa86e6",
        );
        assert_eq!(
            cargo_profile_dir_of(exe),
            Some(PathBuf::from("/w/target/release"))
        );
    }

    #[test]
    fn the_nearest_folder_wins_over_one_named_build_further_up() {
        let exe = Path::new("/home/build/repo/target/debug/deps/t-1");
        assert_eq!(
            cargo_profile_dir_of(exe),
            Some(PathBuf::from("/home/build/repo/target/debug"))
        );
    }

    #[test]
    fn an_executable_outside_a_profile_has_none() {
        assert_eq!(cargo_profile_dir_of(Path::new("/tmp/copied/t-1")), None);
    }

    /// A separate build folder (as on build servers that set
    /// `CARGO_BUILD_BUILD_DIR`) puts the test executable under
    /// `<build-dir>/debug/build/...` while the binaries stay in
    /// `<target>/debug`.
    #[test]
    fn a_separate_build_folder_resolves_to_the_target_profile() {
        let root = std::env::temp_dir().join(format!("ck-profile-{}", std::process::id()));
        let target = root.join("target");
        std::fs::create_dir_all(target.join("debug")).unwrap();
        let built = root.join("target/bd/56/ca1d9801768bbc/debug");
        assert_eq!(
            target_profile_dir(&built, Some(&target)),
            Some(target.join("debug"))
        );
        // No release profile was built in the target folder: keep the build one.
        let release = root.join("target/bd/56/ca1d9801768bbc/release");
        assert_eq!(target_profile_dir(&release, Some(&target)), None);
        assert_eq!(target_profile_dir(&built, None), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_running_test_reports_its_own_or_the_target_profile_folder() {
        let exe = std::env::current_exe().unwrap();
        let profile = cargo_profile_dir();
        let built = cargo_profile_dir_of(&exe).unwrap();
        assert!(
            exe.starts_with(&profile)
                || Some(profile.clone())
                    == target_profile_dir(&built, target_dir_from_env().as_deref()),
            "{} is neither above {} nor its target folder's profile",
            profile.display(),
            exe.display()
        );
        // A fresh Cargo 1.100 build folder has build/ but no deps/, and an
        // older one has deps/; either marks a real profile folder.
        assert!(
            built.join("deps").is_dir() || built.join("build").is_dir(),
            "{} has neither deps/ nor build/",
            built.display()
        );
    }
}
