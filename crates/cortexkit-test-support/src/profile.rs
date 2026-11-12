//! Finding the Cargo profile folder a test was built into.
//!
//! Tests that run a workspace `[[bin]]` (a stub module, a fixture helper, the
//! daemon itself) look for it in the profile folder, `target/debug` or
//! `target/release`, where `cargo build` and `cargo test` place binaries.
//! Cargo up to 1.99 built test executables in `<profile>/deps/`, so tests
//! reached the profile folder by going two folders up from their own path.
//! Cargo 1.100 builds them in `<profile>/build/<package>/<hash>/out/`, and the
//! same two steps now land inside the build folder. These functions work for
//! both layouts.

use std::path::{Path, PathBuf};

/// The profile folder for a test executable at `exe`: the parent of the
/// nearest enclosing `deps` or `build` folder. `None` when `exe` is in
/// neither, as when it was copied out of the target folder.
///
/// The binaries are only there when Cargo's build folder is its target folder,
/// which is the default. A separate `build.build-dir` keeps the test
/// executables apart from the binaries, and no path from the executable leads
/// to them.
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

/// The profile folder of the running test executable. See
/// [`cargo_profile_dir_of`].
///
/// Panics when the executable's path can't be read or isn't inside a profile
/// folder, because every caller would otherwise look for its binaries in the
/// wrong place and fail with a less helpful message.
pub fn cargo_profile_dir() -> PathBuf {
    let exe = std::env::current_exe().expect("the test executable's path is readable");
    cargo_profile_dir_of(&exe).unwrap_or_else(|| {
        panic!(
            "test executable {} is not inside a Cargo profile's deps/ or build/ folder",
            exe.display()
        )
    })
}

/// A workspace binary named `name` in the running test's profile folder, with
/// the platform's executable suffix (`.exe` on Windows). The file may not
/// exist; callers check, so they can say which build step makes it.
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

    #[test]
    fn the_running_test_is_inside_the_folder_it_reports() {
        let exe = std::env::current_exe().unwrap();
        let profile = cargo_profile_dir();
        assert!(
            exe.starts_with(&profile),
            "{} is not under {}",
            exe.display(),
            profile.display()
        );
        assert!(
            profile.join("deps").is_dir(),
            "{} has no deps/ folder",
            profile.display()
        );
    }
}
