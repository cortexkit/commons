//! Run named conformance rules against a module's Unix doctor program.

#![cfg(unix)]

mod fix_rules;
mod scratch;
mod stand_in;

use std::io;
use std::path::{Path, PathBuf};

use cortexkit_doctor::{runner, DecodeError, FindingId, Mode};
use scratch::Scratch;

/// Supplies a program and independent test state for each conformance run.
pub trait Adapter {
    fn program(&self) -> &Path;

    /// Populate the two empty scratch homes. Leave the homes themselves as directories.
    fn seed(&mut self, data: &Path, config: &Path) -> io::Result<()>;

    fn fixable_id(&self) -> Option<&FindingId> {
        None
    }

    /// A fixable finding reported by the full check but not by the quick check.
    fn full_fixable_id(&self) -> Option<&FindingId> {
        None
    }

    fn needs_stop_id(&self) -> Option<&FindingId> {
        None
    }

    fn liveness(&self) -> Option<&Liveness> {
        None
    }

    /// Declare that the module test build injects `NoPresence` under the kit's switches.
    /// Do not opt in for a program that can use an OS prompt during a kit fix.
    fn non_prompting(&self) -> bool {
        false
    }
}

/// How the module detects that an instance is already running. Paths are
/// relative to the scratch data home and must not contain `..`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Liveness {
    PidFile(PathBuf),
    FlockFile(PathBuf),
}

/// A rule's result. Reasons are fixed codes and never include program output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    Passed,
    Failed { reason: &'static str },
    Skipped { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RuleResult {
    pub name: &'static str,
    pub outcome: Outcome,
}

/// Run the kit and report each rule by name, including skipped rules.
pub fn run(adapter: &mut impl Adapter) -> Vec<RuleResult> {
    let check = Scratch::new(adapter).and_then(|scratch| {
        runner::check(adapter.program(), Mode::Quick, &scratch.runner_options())
            .map_err(run_failure)
    });
    let read_only = check_read_only(adapter, Mode::Quick);
    let full = check_read_only(adapter, Mode::Full);
    // These two rules inspect different stages of the same manifest invocation.
    let manifest = Scratch::new(adapter).and_then(|scratch| {
        match runner::manifest(adapter.program(), &scratch.runner_options()) {
            Ok(_) => Ok(Outcome::Passed),
            Err(runner::RunError::InvalidOutput(DecodeError::TemplatesDigest)) => {
                Ok(Outcome::Failed {
                    reason: DecodeError::TemplatesDigest.as_str(),
                })
            }
            Err(error) => Err(run_failure(error)),
        }
    });
    let (valid, digest) = match manifest {
        Ok(digest) => (Outcome::Passed, digest),
        Err(reason) => (
            Outcome::Failed { reason },
            Outcome::Skipped {
                reason: "prerequisite_failed",
            },
        ),
    };
    let fixes = fix_rules::run(adapter, check.is_ok(), full.is_ok());
    let mut results = vec![
        RuleResult {
            name: "output.valid",
            outcome: verdict(check),
        },
        RuleResult {
            name: "check.read_only",
            outcome: verdict(read_only),
        },
        RuleResult {
            name: "check.full",
            outcome: verdict(full),
        },
    ];
    results.extend(fixes);
    results.extend([
        RuleResult {
            name: "manifest.valid",
            outcome: valid,
        },
        RuleResult {
            name: "manifest.digest",
            outcome: digest,
        },
    ]);
    results
}

fn check_read_only(adapter: &mut impl Adapter, mode: Mode) -> Result<(), &'static str> {
    let scratch = Scratch::new(adapter)?;
    let before = scratch.snapshot()?;
    let check = runner::check(adapter.program(), mode, &scratch.runner_options());
    let after = scratch.snapshot()?;
    if before != after {
        return Err("home_changed");
    }
    if mode == Mode::Full {
        return check.map(|_| ()).map_err(run_failure);
    }
    // Malformed stdout does not erase a measurement, but a check that never
    // launched cannot demonstrate that the program leaves both homes alone.
    match check {
        Err(error @ (runner::RunError::LaunchFailed(_) | runner::RunError::HomeRelative)) => {
            Err(run_failure(error))
        }
        _ => Ok(()),
    }
}

fn verdict<T>(result: Result<T, &'static str>) -> Outcome {
    match result {
        Ok(_) => Outcome::Passed,
        Err(reason) => Outcome::Failed { reason },
    }
}

fn run_failure(error: runner::RunError) -> &'static str {
    match error {
        runner::RunError::InvalidOutput(stage) => stage.as_str(),
        error => error.as_str(),
    }
}
