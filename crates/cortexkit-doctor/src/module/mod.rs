//! Unix implementation of a module's `doctor` subcommand.

mod confirmation;
mod terminal;

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::ExitCode;

use serde::Serialize;

use crate::{
    is_digest, CheckOutput, DecodeError, Evidence, Finding, FindingId, FixOutput, Manifest,
    ModuleId, MAX_OUTPUT_BYTES,
};

pub use confirmation::{Hooks, Presence, PublishWithdraw, Timing};
pub use subc_presence::{LocalAuthenticationFailure, Outcome, Withdraw};
pub use terminal::{Terminal, TerminalRead};

/// A check could not complete. No error detail is written to stdout.
#[derive(Debug, Clone, Copy)]
pub struct CheckError;

/// A preflight refusal or failure, before asking for confirmation or applying a fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum PreflightError {
    ModuleRunning,
    FilesInUse,
    Failed,
}

/// The refusal codes a fix callback may return without writing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FixRefusal {
    ModuleRunning,
    FilesInUse,
    PlanChanged,
}

/// A validated result from the fix callback.
#[derive(Debug, Clone, PartialEq)]
pub struct FixOutcome(FixOutput);

impl FixOutcome {
    pub fn fixed(evidence: Vec<Evidence>) -> Result<Self, DecodeError> {
        Self::with_evidence(FixOutput::fixed(), evidence)
    }

    pub fn refused(refusal: FixRefusal) -> Self {
        Self(
            Stop::Refused(match refusal {
                FixRefusal::ModuleRunning => "module_running",
                FixRefusal::FilesInUse => "files_in_use",
                FixRefusal::PlanChanged => "plan_changed",
            })
            .output(),
        )
    }

    pub fn failed(code: impl Into<String>, evidence: Vec<Evidence>) -> Result<Self, DecodeError> {
        Self::with_evidence(FixOutput::failed(code)?, evidence)
    }

    fn with_evidence(output: FixOutput, evidence: Vec<Evidence>) -> Result<Self, DecodeError> {
        if evidence.is_empty() {
            Ok(Self(output))
        } else {
            output.with_evidence(evidence).map(Self)
        }
    }
}

/// Serve the arguments following `doctor`. Return the exit code from `main`.
///
/// Dispatch before configuring logging or opening stores. This permanently
/// redirects process stdout to `/dev/null` and writes the document on a saved
/// descriptor. It is a process entry point, not an in-process query API.
pub fn run(
    args: &[impl AsRef<OsStr>],
    check: impl FnMut() -> Result<Vec<Finding>, CheckError>,
    preflight: impl FnMut(&Finding) -> Result<(), PreflightError>,
    fix: impl FnOnce(Finding) -> FixOutcome,
    manifest: &str,
) -> ExitCode {
    run_with(args, check, preflight, fix, manifest, Hooks::default())
}

/// Process entry point with explicit test hooks. Like [`run`], owns stdout.
///
/// Production code must call [`run`]. To test a module with
/// `cortexkit-doctor-conformance`, add a test-only switch to the module binary
/// that calls this entry point only when the kit's marker is set. Compile the
/// switch out of release builds.
pub fn run_with(
    args: &[impl AsRef<OsStr>],
    mut check: impl FnMut() -> Result<Vec<Finding>, CheckError>,
    mut preflight: impl FnMut(&Finding) -> Result<(), PreflightError>,
    fix: impl FnOnce(Finding) -> FixOutcome,
    manifest: &str,
    hooks: Hooks<'_>,
) -> ExitCode {
    let entry = (hooks.now)();
    let Ok(mut stdout) = take_stdout() else {
        return ExitCode::FAILURE;
    };
    let Some(command) = Command::parse(args) else {
        return ExitCode::from(2);
    };
    let Ok(decoded) = Manifest::decode(manifest.as_bytes()) else {
        return ExitCode::FAILURE;
    };
    if !decoded
        .to_canonical_bytes()
        .is_ok_and(|bytes| bytes == manifest.as_bytes())
    {
        return ExitCode::FAILURE;
    }

    let written = match command {
        Command::Manifest => stdout.write_all(manifest.as_bytes()).is_ok(),
        Command::Check => {
            checked(&mut check).is_ok_and(|(_, bytes)| stdout.write_all(&bytes).is_ok())
        }
        Command::Fix(id, digest) => {
            let request = FixRequest {
                id: &id,
                digest,
                module_id: decoded.module_id(),
                entry,
            };
            let output = request
                .apply(&mut check, &mut preflight, fix, &hooks)
                .unwrap_or_else(Stop::output);
            json_line(&output).is_some_and(|bytes| stdout.write_all(&bytes).is_ok())
        }
    };
    if written {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

enum Command<'a> {
    Manifest,
    Check,
    Fix(FindingId, &'a str),
}

impl<'a> Command<'a> {
    fn parse(args: &'a [impl AsRef<OsStr>]) -> Option<Self> {
        match args {
            [cmd] if cmd.as_ref() == "manifest" => Some(Self::Manifest),
            [cmd, json] if cmd.as_ref() == "check" && json.as_ref() == "--json" => {
                Some(Self::Check)
            }
            [cmd, id, flag, digest] if cmd.as_ref() == "fix" && flag.as_ref() == "--digest" => {
                let id = id.as_ref().to_str()?.parse().ok()?;
                let digest = digest.as_ref().to_str()?;
                is_digest(digest).then_some(Self::Fix(id, digest))
            }
            _ => None,
        }
    }
}

fn take_stdout() -> io::Result<File> {
    // SAFETY: fcntl creates a new owned descriptor without changing fd 1.
    let saved = retry_fd(|| unsafe { libc::fcntl(libc::STDOUT_FILENO, libc::F_DUPFD_CLOEXEC, 3) })?;
    // SAFETY: saved came from F_DUPFD_CLOEXEC and has no other owner.
    let saved = unsafe { File::from_raw_fd(saved) };
    let null = OpenOptions::new().write(true).open("/dev/null")?;
    // SAFETY: null stays open; dup2 atomically replaces process stdout.
    retry_fd(|| unsafe { libc::dup2(null.as_raw_fd(), libc::STDOUT_FILENO) })?;
    Ok(saved)
}

fn retry_fd(mut call: impl FnMut() -> libc::c_int) -> io::Result<libc::c_int> {
    loop {
        let fd = call();
        if fd >= 0 {
            return Ok(fd);
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::Interrupted {
            return Err(err);
        }
    }
}

fn json_line(value: &impl Serialize) -> Option<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).ok()?;
    if bytes.len() >= MAX_OUTPUT_BYTES {
        return None;
    }
    bytes.push(b'\n');
    Some(bytes)
}

#[derive(Debug)]
enum Stop {
    Refused(&'static str),
    Failed(&'static str),
}

impl Stop {
    fn output(self) -> FixOutput {
        match self {
            Self::Refused(code) => FixOutput::refused(code),
            Self::Failed(code) => FixOutput::failed(code),
        }
        .expect("helper codes are fixed, valid strings")
    }
}

fn callback<T>(call: impl FnOnce() -> T) -> Result<T, Stop> {
    catch_unwind(AssertUnwindSafe(call)).map_err(|payload| {
        // A custom panic payload can itself panic on drop.
        std::mem::forget(payload);
        Stop::Failed("callback_panicked")
    })
}

fn checked(
    check: impl FnOnce() -> Result<Vec<Finding>, CheckError>,
) -> Result<(CheckOutput, Vec<u8>), Stop> {
    let findings = callback(check)?.map_err(|_| Stop::Failed("check_failed"))?;
    let output = CheckOutput::new(findings).map_err(|_| Stop::Failed("check_failed"))?;
    let bytes = json_line(&output).ok_or(Stop::Failed("check_failed"))?;
    Ok((output, bytes))
}

fn preflight_verified(
    preflight: &mut impl FnMut(&Finding) -> Result<(), PreflightError>,
    finding: &Finding,
) -> Result<(), Stop> {
    callback(|| preflight(finding))?.map_err(|err| match err {
        PreflightError::ModuleRunning => Stop::Refused("module_running"),
        PreflightError::FilesInUse => Stop::Refused("files_in_use"),
        PreflightError::Failed => Stop::Failed("preflight_failed"),
    })
}

struct FixRequest<'a> {
    id: &'a FindingId,
    digest: &'a str,
    module_id: &'a ModuleId,
    entry: std::time::Instant,
}

impl FixRequest<'_> {
    fn apply(
        self,
        check: &mut impl FnMut() -> Result<Vec<Finding>, CheckError>,
        preflight: &mut impl FnMut(&Finding) -> Result<(), PreflightError>,
        fix: impl FnOnce(Finding) -> FixOutcome,
        hooks: &Hooks<'_>,
    ) -> Result<FixOutput, Stop> {
        let first = self.check_finding(check)?;
        preflight_verified(preflight, &first)?;
        confirmation::confirm(hooks, self.entry, self.module_id, &first)?;

        let current = self.check_finding(check)?;
        if current.summary() != first.summary() {
            return Err(Stop::Refused("plan_changed"));
        }
        preflight_verified(preflight, &first)?;
        if (hooks.now)().duration_since(self.entry) > hooks.timing.fix_gate {
            return Err(Stop::Refused("out_of_time"));
        }
        callback(|| fix(first)).map(|result| result.0)
    }

    fn check_finding(
        &self,
        check: &mut impl FnMut() -> Result<Vec<Finding>, CheckError>,
    ) -> Result<Finding, Stop> {
        let (output, _) = checked(check)?;
        let finding = output
            .findings()
            .iter()
            .find(|finding| finding.id() == self.id)
            .ok_or(Stop::Refused("unknown_finding"))?;
        if !finding
            .fix()
            .plan()
            .is_some_and(|plan| plan.digest() == self.digest)
        {
            return Err(Stop::Refused("plan_changed"));
        }
        Ok(finding.clone())
    }
}

#[cfg(test)]
mod tests;
