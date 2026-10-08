use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ROOTS: &[(&str, &str)] = &[
    ("HOME", "home"),
    ("XDG_DATA_HOME", "data"),
    ("XDG_RUNTIME_DIR", "runtime"),
    ("XDG_CONFIG_HOME", "config"),
    ("XDG_STATE_HOME", "state"),
    ("XDG_CACHE_HOME", "cache"),
    ("TMPDIR", "tmp"),
];
const PASSTHROUGH: &[&str] = &[
    "PATH",
    "LANG",
    "LANGUAGE",
    "LC_ALL",
    "LC_CTYPE",
    "LC_COLLATE",
    "LC_MESSAGES",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_MONETARY",
    "TZ",
    "USER",
    "LOGNAME",
    "RUST_BACKTRACE",
    #[cfg(windows)]
    "SystemRoot",
];

/// Return the scratch subdirectory assigned to a built-in directory variable,
/// such as `XDG_DATA_HOME`. Unknown variable names panic rather than resolving
/// to a directory that the child never uses.
pub fn fenced_subc_env_dir(scratch: &Path, var: &str) -> PathBuf {
    ROOTS
        .iter()
        .find(|(name, _)| *name == var)
        .map(|(_, dir)| scratch.join(dir))
        .unwrap_or_else(|| panic!("{var} is not a fenced daemon env root"))
}

/// Create directories beneath scratch and return their environment assignments.
/// HOME, all five XDG directory variables, and TMPDIR point into this test tree
/// so a daemon and its children do not write to the user's normal directories.
pub fn fenced_subc_daemon_env(scratch: &Path) -> Vec<(OsString, OsString)> {
    fenced_env_with_roots(scratch, &[])
}

pub(crate) fn fenced_env_with_roots(
    scratch: &Path,
    extra_roots: &[(&str, &str)],
) -> Vec<(OsString, OsString)> {
    let mut names = std::collections::HashSet::new();
    for (name, directory) in extra_roots {
        assert!(
            !name.is_empty()
                && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                && !name.as_bytes()[0].is_ascii_digit(),
            "extra root must have a valid environment variable name"
        );
        assert!(
            !ROOTS
                .iter()
                .any(|(builtin, _)| name.eq_ignore_ascii_case(builtin)),
            "extra roots cannot replace a built-in directory variable"
        );
        assert!(
            names.insert(name.to_ascii_uppercase()),
            "duplicate extra root variable"
        );
        plain_name(directory);
    }
    ROOTS
        .iter()
        .chain(extra_roots)
        .map(|(name, dir)| {
            let path = scratch.join(dir);
            std::fs::create_dir_all(&path).unwrap_or_else(|e| panic!("create fenced {name}: {e}"));
            let metadata = std::fs::symlink_metadata(&path).expect("inspect test directory root");
            assert!(
                metadata.is_dir() && !metadata.is_symlink(),
                "test directory root must be a real directory"
            );
            (OsString::from(name), path.into_os_string())
        })
        .collect()
}

/// Build a command with no inherited environment except the allowlisted PATH,
/// locale, timezone, user-name, and backtrace variables. Inherited API keys,
/// tokens, and module identity variables are removed; HOME, XDG directories,
/// and TMPDIR instead point beneath scratch. A `ck-*` program is copied to a
/// reusable `ckdev-*` executable before the command is constructed.
/// Use [`crate::TestDaemonCommand`] when roots must also resist later overrides
/// and the child must terminate with the test harness.
pub fn fenced_command(program: impl AsRef<OsStr>, scratch: &Path) -> Command {
    fenced_command_with_roots(program, scratch, &[])
}

/// Like [`fenced_command`], with caller-selected directory variables redirected
/// beneath scratch. Each pair is `(variable, plain subdirectory name)`; built-in
/// variables cannot be replaced. Invalid names, duplicates, and symlink roots panic.
/// As with a standard `Command`, later `.env` calls can override these values;
/// [`crate::TestDaemonCommand::fenced_root`] enforces them again when spawning.
pub fn fenced_command_with_roots(
    program: impl AsRef<OsStr>,
    scratch: &Path,
    extra_roots: &[(&str, &str)],
) -> Command {
    let program = crate::stage_test_binary(Path::new(program.as_ref()));
    let mut command = crate::dev_command(program);
    command.env_clear();
    for name in PASSTHROUGH {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command.envs(fenced_env_with_roots(scratch, extra_roots));
    command
}

/// Run `--version` with the same environment fence used by test daemons.
/// Assessment is best-effort; failed exits are reported with code or signal.
pub fn warm_exec_fenced(binary: &Path) {
    let scratch = crate::ScratchDir::new("warm-exec");
    if let Err(error) = crate::checked_output(
        fenced_command(binary, scratch.path())
            .arg("--version")
            .stderr(Stdio::null()),
    ) {
        eprintln!("warm-exec: {error}");
    }
}

/// Best-effort macOS signing. The explicit identity wins over
/// `CORTEXKIT_TEST_SIGNING_IDENTITY`; without either, signing is skipped.
/// Pass `Some(OsStr::new("-"))` to select ad-hoc signing without a certificate
/// or a signing identity from the user's keychain.
/// Already-valid signatures are left alone; other platforms do nothing.
pub fn sign_test_binary(path: &Path, identity: Option<&OsStr>) {
    #[cfg(target_os = "macos")]
    {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let env = std::env::var_os("CORTEXKIT_TEST_SIGNING_IDENTITY");
        let Some(identity) = identity.or(env.as_deref()) else {
            return;
        };
        if Command::new("codesign")
            .arg("-v")
            .arg(path)
            .output()
            .is_ok_and(|o| o.status.success())
        {
            return;
        }
        if let Err(e) = crate::checked_output(
            Command::new("codesign")
                .args(["-f", "-s"])
                .arg(identity)
                .arg(path),
        ) {
            eprintln!("sign-test-binary: {e}; running {} anyway", path.display());
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (path, identity);
}

/// Build with a locked sibling manifest and an isolated, caller-owned target directory.
pub fn sibling_cargo_build(repo: &Path, target_dir: &Path, bin: &str) -> Command {
    let mut command = Command::new(env!("CARGO"));
    command
        .current_dir(repo)
        .args(["build", "--locked", "--bin", bin])
        .env("CARGO_TARGET_DIR", target_dir);
    command
}

fn workspace_root() -> PathBuf {
    if let Some(root) = std::env::var_os("CORTEXKIT_TEST_WORKSPACE_ROOT") {
        return PathBuf::from(root);
    }
    // Resolve at runtime rather than from this library's manifest: consumers
    // may use a registry dependency whose sources live outside their workspace.
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .expect("locate test workspace");
    assert!(
        output.status.success(),
        "set CORTEXKIT_TEST_WORKSPACE_ROOT outside a git workspace"
    );
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}
pub(crate) fn plain_name(name: &str) {
    assert!(
        !name.is_empty()
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
        "expected a plain directory name (ASCII letters, digits, '-' or '_')"
    );
}
/// Caller workspace's `target/sibling-target/<name>`; never writes into the sibling.
pub fn sibling_target_dir(name: &str) -> PathBuf {
    plain_name(name);
    let dir = workspace_root().join("target/sibling-target").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
/// Adjacent checkout, falling back to the primary checkout's sibling for linked worktrees.
/// `CORTEXKIT_TEST_WORKSPACE_ROOT` can explicitly select the consuming workspace.
pub fn sibling_checkout(name: &str) -> Result<PathBuf, String> {
    plain_name(name);
    let workspace = workspace_root();
    let direct = workspace.join("..").join(name);
    let primary = || {
        let marker = std::fs::read_to_string(workspace.join(".git")).ok()?;
        let git_dir = workspace.join(marker.trim().strip_prefix("gitdir: ")?);
        let primary = git_dir
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == ".git"))?
            .parent()?;
        Some(primary.parent()?.join(name))
    };
    [Some(direct.clone()), primary()]
        .into_iter()
        .flatten()
        .find(|p| p.exists())
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| {
            format!(
                "sibling {name} checkout missing (looked at {})",
                direct.display()
            )
        })
}

/// The platform daemon `ck-subc`, built from the `subconscious` sibling checkout.
/// Its executable path is private so callers can only spawn it through a builder
/// that redirects its directory variables and owns its process lifetime.
#[derive(Debug, Clone)]
pub struct SubcDaemonBinary {
    path: PathBuf,
}
impl SubcDaemonBinary {
    pub fn build(dependency_checkouts: &[&str]) -> Self {
        Self::try_build(dependency_checkouts).unwrap_or_else(|reason| panic!("{reason}"))
    }
    /// Missing checkout is recoverable; a failed build is a test failure.
    /// `dependency_checkouts` names the daemon's adjacent path dependencies
    /// whose source changes should invalidate the executable cache.
    pub fn try_build(dependency_checkouts: &[&str]) -> Result<Self, String> {
        let repo = sibling_checkout("subconscious")?;
        Ok(Self {
            path: crate::build_sibling_binary(
                &repo,
                "subconscious",
                "ck-subc",
                dependency_checkouts,
            ),
        })
    }
    pub fn command(&self, scratch: &Path) -> crate::TestDaemonCommand {
        crate::TestDaemonCommand::new(&self.path, scratch)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn caller_selected_root_lands_under_scratch() {
        let scratch = crate::ScratchDir::new("extra-command-root");
        let output = crate::checked_output(&mut fenced_command_with_roots(
            "/usr/bin/env",
            &scratch,
            &[("EXAMPLE_STORAGE_ROOT", "storage")],
        ))
        .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text
            .lines()
            .any(|line| line
                == format!("EXAMPLE_STORAGE_ROOT={}", scratch.join("storage").display())));
        assert!(scratch.join("storage").is_dir());
    }

    #[test]
    fn extra_roots_cannot_replace_builtin_variables_or_escape_scratch() {
        let scratch = crate::ScratchDir::new("extra-root-validation");
        for roots in [
            vec![("XDG_DATA_HOME", "other")],
            vec![("xdg_data_home", "other")],
            vec![("EXTRA_ROOT", "..")],
            vec![("EXTRA_ROOT", "/outside")],
            vec![("EXTRA_ROOT", "../outside")],
            vec![("BAD=NAME", "data")],
            vec![("EXTRA_ROOT", "first"), ("EXTRA_ROOT", "second")],
        ] {
            assert!(
                std::panic::catch_unwind(|| fenced_command_with_roots("env", &scratch, &roots))
                    .is_err(),
                "accepted invalid roots: {roots:?}"
            );
        }
        let outside = crate::ScratchDir::new("extra-root-outside");
        std::os::unix::fs::symlink(outside.path(), scratch.join("linked")).unwrap();
        assert!(std::panic::catch_unwind(|| fenced_command_with_roots(
            "env",
            &scratch,
            &[("EXTRA_ROOT", "linked")]
        ))
        .is_err());
    }

    #[test]
    fn fenced_command_drops_ambient_variables_and_keeps_the_allowlist() {
        let scratch = crate::ScratchDir::new("fence-env");
        std::env::set_var("CORTEXKIT_TEST_LEAK_CANARY", "secret");
        let output =
            crate::checked_output(fenced_command("/usr/bin/env", &scratch).env("EXPLICIT", "kept"))
                .unwrap();
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("CORTEXKIT_TEST_LEAK_CANARY="));
        assert!(text.lines().any(|s| s == "EXPLICIT=kept"));
        assert!(text
            .lines()
            .any(|s| s == format!("PATH={}", std::env::var("PATH").unwrap())));
        for (name, dir) in ROOTS {
            assert!(text
                .lines()
                .any(|s| s == format!("{name}={}", scratch.join(dir).display())));
        }
        for line in text.lines() {
            let name = line.split_once('=').unwrap().0;
            assert!(
                ROOTS.iter().any(|(root, _)| *root == name)
                    || PASSTHROUGH.contains(&name)
                    || name == "EXPLICIT",
                "unexpected variable {name}"
            );
        }
        std::env::remove_var("CORTEXKIT_TEST_LEAK_CANARY");
    }
    #[test]
    fn sibling_cargo_build_fails_on_a_stale_lock_and_leaves_it_untouched() {
        let scratch = crate::ScratchDir::new("locked-build");
        std::fs::write(scratch.join("Cargo.toml"), "[package]\nname='lockprobe'\nversion='0.2.0'\nedition='2021'\n[[bin]]\nname='lockprobe'\npath='main.rs'\n[workspace]\n").unwrap();
        std::fs::write(scratch.join("main.rs"), "fn main() {}\n").unwrap();
        let lock = |version: &str| {
            format!("version = 3\n\n[[package]]\nname = \"lockprobe\"\nversion = \"{version}\"\n")
        };
        std::fs::write(scratch.join("Cargo.lock"), lock("0.1.0")).unwrap();
        let build = || {
            sibling_cargo_build(&scratch, &scratch.join("target"), "lockprobe")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .unwrap()
        };
        assert!(!build().success());
        assert_eq!(
            std::fs::read_to_string(scratch.join("Cargo.lock")).unwrap(),
            lock("0.1.0")
        );
        std::fs::write(scratch.join("Cargo.lock"), lock("0.2.0")).unwrap();
        assert!(build().success());
    }
}
