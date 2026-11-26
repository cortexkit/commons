//! Run a module's doctor commands as isolated Unix processes.
//!
//! The runner owns waiting for its direct child. Callers must not reap it from
//! a separate SIGCHLD handler or configure the OS to reap children automatically.
//! Serialize foreground fixes on a given terminal.

mod process;

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use cortexkit_store_types::{resolve_config_home, resolve_data_home};

use crate::{CheckOutput, DecodeError, FindingId, FixOutput, Manifest};
pub use crate::{Crash, RunError};

/// Process deadlines and cleanup intervals. Keep the contract defaults outside tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    pub check: Duration,
    pub manifest: Duration,
    /// Extra time on top of the supplied plan's runtime.
    pub fix_allowance: Duration,
    pub exit_grace: Duration,
    pub term_to_kill: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            check: Duration::from_secs(10),
            manifest: Duration::from_secs(2),
            fix_allowance: Duration::from_secs(150),
            exit_grace: Duration::from_secs(2),
            term_to_kill: Duration::from_secs(2),
        }
    }
}

impl Limits {
    /// The full process deadline for a plan with the given `timeout_ms`.
    pub fn fix_timeout(&self, timeout_ms: u64) -> Duration {
        self.fix_allowance
            .saturating_add(Duration::from_millis(timeout_ms))
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

    /// Disable terminal handoff for fixes, even if the runner is foreground.
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

/// Invoke `<program> doctor check --json` and validate its output.
pub fn check(program: impl AsRef<Path>, options: &Options) -> Result<CheckOutput, RunError> {
    invoke(
        program.as_ref(),
        &["check", "--json"],
        options,
        options.limits.check,
        CheckOutput::decode,
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

/// Invoke `<program> doctor fix <id> --digest <digest>` and validate its output.
/// Pass the selected plan's digest and `timeout_ms` from the preceding check.
pub fn fix(
    program: impl AsRef<Path>,
    id: &FindingId,
    digest: &str,
    timeout_ms: u64,
    options: &Options,
) -> Result<FixOutput, RunError> {
    invoke(
        program.as_ref(),
        &["fix", id.as_str(), "--digest", digest],
        options,
        options.limits.fix_timeout(timeout_ms),
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
#[cfg(not(target_os = "linux"))]
const PROMPT_VARS: &[&str] = &[];

fn invoke<T>(
    program: &Path,
    args: &[&str],
    options: &Options,
    timeout: Duration,
    decode: impl FnOnce(&[u8]) -> Result<T, DecodeError>,
) -> Result<T, RunError> {
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
        assert_eq!(limits.manifest, Duration::from_secs(2));
        assert_eq!(limits.fix_timeout(1_234), Duration::from_millis(151_234));
        assert_eq!(limits.exit_grace, Duration::from_secs(2));
        assert_eq!(limits.term_to_kill, Duration::from_secs(2));
    }
}
