//! Run a module's doctor commands in isolated process trees.
//!
//! The runner owns waiting for its direct child. On Unix, callers must not
//! reap it from a separate SIGCHLD handler or enable automatic reaping.
//! Serialize fixes that share a terminal or console.

#[cfg_attr(windows, path = "process/windows.rs")]
mod process;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use cortexkit_store_types::{resolve_config_home, resolve_data_home};

use crate::{
    CheckOutput, DecodeError, FindingId, FixOutput, Manifest, Mode, CONFIRMATION_LIMIT,
    FULL_CHECK_LIMIT, QUICK_CHECK_LIMIT,
};
pub use crate::{Crash, RunError};

// Time to enter the helper and, after the fix, write its result and exit.
const FIX_IO_ALLOWANCE: Duration = Duration::from_secs(10);

/// Process deadlines and cleanup intervals. Keep the contract defaults outside tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    pub check: Duration,
    pub full_check: Duration,
    pub manifest: Duration,
    /// Extra time on top of the supplied plan's runtime for a quick-mode fix.
    pub fix_allowance: Duration,
    pub exit_grace: Duration,
    pub term_to_kill: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            check: QUICK_CHECK_LIMIT,
            full_check: FULL_CHECK_LIMIT,
            manifest: Duration::from_secs(2),
            fix_allowance: FIX_IO_ALLOWANCE + CONFIRMATION_LIMIT + QUICK_CHECK_LIMIT * 2,
            exit_grace: Duration::from_secs(2),
            term_to_kill: Duration::from_secs(2),
        }
    }
}

impl Limits {
    /// The full process deadline for a plan with the given `timeout_ms`.
    pub fn fix_timeout(&self, timeout_ms: u64, mode: Mode) -> Duration {
        let allowance = match mode {
            Mode::Quick => self.fix_allowance,
            Mode::Full => self
                .full_check
                .saturating_mul(2)
                .saturating_add(CONFIRMATION_LIMIT + FIX_IO_ALLOWANCE),
        };
        allowance.saturating_add(Duration::from_millis(timeout_ms))
    }
}

/// Caller-supplied homes, terminal policy and deadlines for a doctor run.
#[cfg_attr(
    not(feature = "conformance"),
    doc = "\nThe conformance fix switch is unavailable without the `conformance` feature.\n\n```compile_fail,E0599\nuse cortexkit_doctor::runner::Options;\nlet _ = Options::new().with_conformance_fix_marker();\n```"
)]
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Options {
    data_home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    force_setsid: bool,
    #[cfg(feature = "conformance")]
    conformance_fix: bool,
    limits: Limits,
}

impl Options {
    /// Resolve homes from the current environment and use the contract deadlines.
    pub fn new() -> Self {
        Self::default()
    }

    /// Supply the absolute `XDG_DATA_HOME` instead of resolving it.
    pub fn with_data_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.data_home = Some(home.into());
        self
    }

    /// Supply the absolute `XDG_CONFIG_HOME` instead of resolving it.
    pub fn with_config_home(mut self, home: impl Into<PathBuf>) -> Self {
        self.config_home = Some(home.into());
        self
    }

    /// Disable terminal or console access for fixes.
    pub fn with_force_setsid(mut self, force_setsid: bool) -> Self {
        self.force_setsid = force_setsid;
        self
    }

    /// For fixes only: force a new session and pass the conformance test switches
    /// instead of the OS prompt environment. Only use with a test build that
    /// injects `NoPresence` when both switches match.
    #[cfg(feature = "conformance")]
    pub fn with_conformance_fix_marker(mut self) -> Self {
        self.conformance_fix = true;
        self
    }

    /// Override the deadlines. Callers other than tests should keep the defaults.
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    fn configure_fix(&self, command: &mut Command) -> bool {
        #[cfg(feature = "conformance")]
        if self.conformance_fix {
            command
                .env("CORTEXKIT_DOCTOR_CONFORMANCE", "1")
                .env("CORTEXKIT_DOCTOR_TEST_PRESENCE", "no_presence");
            return false;
        }
        command.envs(
            PROMPT_VARS
                .iter()
                .filter_map(|&key| std::env::var_os(key).map(|value| (key, value))),
        );
        !self.force_setsid
    }
}

/// Invoke `<program> doctor check [--full] --json` and validate its output.
/// An older helper that rejects `--full` returns [`RunError::NoDoctor`]; this
/// function does not retry in quick mode.
pub fn check(
    program: impl AsRef<Path>,
    mode: Mode,
    options: &Options,
) -> Result<CheckOutput, RunError> {
    let (args, timeout): (&[&str], _) = match mode {
        Mode::Quick => (&["check", "--json"], options.limits.check),
        Mode::Full => (&["check", "--full", "--json"], options.limits.full_check),
    };
    invoke(
        program.as_ref(),
        args,
        options,
        timeout,
        CheckOutput::decode,
    )
}

/// Shared, one-shot cancellation state for [`check_cancellable`].
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// Create a handle that has not been cancelled.
    pub fn new() -> Self {
        Self::default()
    }

    /// Request cancellation from another thread or task. Clones share this state.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// Invoke [`check`] with a caller-controlled cancellation handle.
/// If the handle is cancelled before launch, the helper is not started. If
/// cancellation is observed while the helper is running, its process tree
/// is terminated and the direct child has exited before returning
/// [`RunError::Cancelled`]. An exited helper keeps its result.
pub fn check_cancellable(
    program: impl AsRef<Path>,
    mode: Mode,
    options: &Options,
    cancel: &Cancel,
) -> Result<CheckOutput, RunError> {
    let (args, timeout): (&[&str], _) = match mode {
        Mode::Quick => (&["check", "--json"], options.limits.check),
        Mode::Full => (&["check", "--full", "--json"], options.limits.full_check),
    };
    invoke_with_cancel(
        program.as_ref(),
        args,
        options,
        timeout,
        CheckOutput::decode,
        Some(cancel),
    )
}

/// Invoke `<program> doctor manifest` and validate its output.
pub fn manifest(program: impl AsRef<Path>, options: &Options) -> Result<Manifest, RunError> {
    invoke(
        program.as_ref(),
        &["manifest"],
        options,
        options.limits.manifest,
        Manifest::decode,
    )
}

/// Invoke `<program> doctor fix <id> --digest <digest> [--full]` and validate its output.
/// Pass the selected plan's digest and `timeout_ms`, and use the mode from the
/// check that reported it.
pub fn fix(
    program: impl AsRef<Path>,
    id: &FindingId,
    digest: &str,
    timeout_ms: u64,
    mode: Mode,
    options: &Options,
) -> Result<FixOutput, RunError> {
    let args = ["fix", id.as_str(), "--digest", digest, "--full"];
    let args = match mode {
        Mode::Quick => &args[..4],
        Mode::Full => &args[..],
    };
    invoke(
        program.as_ref(),
        args,
        options,
        options.limits.fix_timeout(timeout_ms, mode),
        FixOutput::decode,
    )
}

#[cfg(target_os = "linux")]
const PROMPT_VARS: &[&str] = &[
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "DISPLAY",
    "WAYLAND_DISPLAY",
];
#[cfg(windows)]
const PROMPT_VARS: &[&str] = &[
    "SystemRoot",
    "windir",
    "USERPROFILE",
    "LOCALAPPDATA",
    "TEMP",
];
#[cfg(not(any(target_os = "linux", windows)))]
const PROMPT_VARS: &[&str] = &[];

fn invoke<T>(
    program: &Path,
    args: &[&str],
    options: &Options,
    timeout: Duration,
    decode: impl FnOnce(&[u8]) -> Result<T, DecodeError>,
) -> Result<T, RunError> {
    invoke_with_cancel(program, args, options, timeout, decode, None)
}

fn invoke_with_cancel<T>(
    program: &Path,
    args: &[&str],
    options: &Options,
    timeout: Duration,
    decode: impl FnOnce(&[u8]) -> Result<T, DecodeError>,
    cancel: Option<&Cancel>,
) -> Result<T, RunError> {
    if cancel.is_some_and(Cancel::is_cancelled) {
        return Err(RunError::Cancelled);
    }
    let data = absolute_home(&options.data_home, resolve_data_home)?;
    let config = absolute_home(&options.config_home, resolve_config_home)?;
    let is_fix = args[0] == "fix";
    let mut command = Command::new(program);
    command
        .arg("doctor")
        .args(args)
        .env_clear()
        .env("XDG_DATA_HOME", data.as_os_str())
        .env("XDG_CONFIG_HOME", config.as_os_str())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped());
    let terminal_handoff = is_fix && options.configure_fix(&mut command);
    let stdout = process::run(
        &mut command,
        terminal_handoff,
        args[0],
        timeout,
        options.limits,
        cancel,
    )?;
    decode(&stdout).map_err(RunError::InvalidOutput)
}

fn absolute_home(
    path: &Option<PathBuf>,
    resolve: fn() -> String,
) -> Result<Cow<'_, Path>, RunError> {
    let path = path
        .as_deref()
        .map_or_else(|| Cow::Owned(PathBuf::from(resolve())), Cow::Borrowed);
    if !path.is_absolute() {
        return Err(RunError::HomeRelative);
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_helper_default_limits() {
        let limits = Limits::default();
        assert_eq!(limits.check, Duration::from_secs(10));
        assert_eq!(limits.full_check, Duration::from_secs(1800));
        assert_eq!(limits.manifest, Duration::from_secs(2));
        assert_eq!(
            limits.fix_timeout(1_234, Mode::Quick),
            Duration::from_millis(151_234)
        );
        assert_eq!(limits.exit_grace, Duration::from_secs(2));
        assert_eq!(limits.term_to_kill, Duration::from_secs(2));
    }

    #[test]
    fn runner_helper_full_fix_timeout_budgets_both_checks() {
        let mut limits = Limits::default();
        assert_eq!(
            limits.fix_timeout(1_234, Mode::Full),
            Duration::from_millis(3_731_234)
        );
        limits.full_check = Duration::from_secs(1);
        assert_eq!(
            limits.fix_timeout(1_234, Mode::Full),
            Duration::from_millis(133_234)
        );
        limits.full_check = Duration::MAX;
        assert_eq!(limits.fix_timeout(1_234, Mode::Full), Duration::MAX);
    }
}
