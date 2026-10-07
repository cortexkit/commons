//! Running the cases and computing the verdict.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use cortexkit_role_harness::{CrashDriver, RealKillVerdict};
use cortexkit_role_llm_runner::points;

use crate::{
    cases::{Case, Ending},
    crash::{self, CrashObservation},
    drive::Mint,
    report::{CaseOutcome, CaseReport, CaseSpec, SuiteReport, SuiteVerdict, CASES},
    retention,
    route::RunnerRoute,
    subject::{Capability, LlmRunnerSubject},
};

/// The run could not start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SetupError {
    /// The work directory is not empty. Each run needs a fresh one, because
    /// a state root must start empty and session names are never reused.
    WorkDirNotEmpty(PathBuf),
    Io(String),
}

impl std::fmt::Display for SetupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WorkDirNotEmpty(path) => write!(f, "{} is not empty", path.display()),
            Self::Io(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SetupError {}

/// Run every case against `subject`, with state roots under `work_dir`,
/// which must be empty or absent.
pub async fn run_suite<S>(subject: &S, work_dir: &Path) -> Result<SuiteReport, SetupError>
where
    S: LlmRunnerSubject,
    S::Route: RunnerRoute,
{
    if work_dir
        .read_dir()
        .map(|mut entries| entries.next().is_some())
        .unwrap_or(false)
    {
        return Err(SetupError::WorkDirNotEmpty(work_dir.to_owned()));
    }
    std::fs::create_dir_all(work_dir.join("main"))
        .map_err(|e| SetupError::Io(format!("creating main: {e}")))?;

    let mut driver = CrashDriver::new(subject);
    let mut declared: BTreeSet<Capability> = subject
        .capabilities()
        .into_iter()
        .filter(|capability| !matches!(capability, Capability::KillAt(_)))
        .collect();
    for declaration in driver.declared_points() {
        if let Some(point) = points::ALL
            .iter()
            .find(|point| **point == declaration.point.as_str())
        {
            declared.insert(Capability::KillAt(point));
        }
    }
    let mint = Mint::default();
    let main = driver
        .spawn(&work_dir.join("main"))
        .await
        .map_err(|e| format!("spawning the runner failed: {e}"));

    let mut crashes: BTreeMap<&'static str, Result<CrashObservation, String>> = BTreeMap::new();
    let mut cases = Vec::with_capacity(CASES.len());
    for spec in CASES {
        let outcome = match gate(spec, &declared) {
            Some(outcome) => outcome,
            None => {
                let result = if spec.name == "crash_at_RetentionTombstoned" {
                    retention::crash(subject, &mut driver, work_dir, &declared, &mint)
                        .await
                        .map(|()| Ending::Passed)
                } else if let Some(point) = crash::point_of(spec.name) {
                    if !crashes.contains_key(point) {
                        let observed =
                            crash::observe(subject, &mut driver, work_dir, &declared, &mint, point)
                                .await;
                        crashes.insert(point, observed);
                    }
                    match &crashes[point] {
                        Err(error) => Err(format!("the {point} crash scenario failed: {error}")),
                        Ok(observed) => crash::check(spec.name, point, observed, &declared)
                            .map(|()| Ending::Passed),
                    }
                } else {
                    match &main {
                        Err(error) => Err(error.clone()),
                        Ok(handle) => {
                            Case {
                                subject,
                                handle,
                                declared: &declared,
                                mint: &mint,
                            }
                            .run(spec.name)
                            .await
                        }
                    }
                };
                match result {
                    Ok(Ending::Passed) => CaseOutcome::Passed,
                    Ok(Ending::Inapplicable(reason)) => CaseOutcome::Inapplicable { reason },
                    Err(reason) => CaseOutcome::Failed { reason },
                }
            }
        };
        cases.push(CaseReport {
            case: spec.name,
            requires: spec
                .requires
                .iter()
                .chain(spec.requires_any)
                .copied()
                .collect(),
            outcome,
        });
    }
    drop(main);

    let kills = driver.ledger().kills().to_vec();
    let mut failures: Vec<String> = cases
        .iter()
        .filter_map(|case| match &case.outcome {
            CaseOutcome::Failed { reason } => Some(format!("{}: {reason}", case.case)),
            _ => None,
        })
        .collect();
    match driver.ledger().verdict() {
        RealKillVerdict::RealKillPresent => {}
        RealKillVerdict::NoKills => failures.push(
            "no kill was made in this run, and a run passes only when at least one kill ended a \
             real process"
                .to_owned(),
        ),
        RealKillVerdict::OnlySimulated => {
            let used: Vec<String> = kills
                .iter()
                .map(|kill| format!("{} by {}", kill.point, kill.mechanism))
                .collect();
            failures.push(format!(
                "no kill in this run ended a real process ({}), so the run cannot tell whether \
                 the runner keeps something only in memory",
                used.join(", ")
            ));
        }
    }
    let skipped: BTreeSet<Capability> = cases
        .iter()
        .filter_map(|case| match &case.outcome {
            CaseOutcome::Skipped { missing } => Some(missing.iter().copied()),
            _ => None,
        })
        .flatten()
        .collect();
    let verdict = if !failures.is_empty() {
        SuiteVerdict::Failed { reasons: failures }
    } else if !skipped.is_empty() {
        SuiteVerdict::ConformingForDeclaredCapabilities {
            declared: declared.into_iter().collect(),
            skipped: skipped.into_iter().collect(),
        }
    } else {
        SuiteVerdict::Passed
    };
    Ok(SuiteReport {
        cases,
        kills,
        verdict,
    })
}

/// The outcome of a case that cannot run against this subject: skipped
/// with what it lacks, or inapplicable. `None` when the case can run.
fn gate(spec: &CaseSpec, declared: &BTreeSet<Capability>) -> Option<CaseOutcome> {
    let mut missing: Vec<Capability> = spec
        .requires
        .iter()
        .filter(|required| !declared.contains(required))
        .copied()
        .collect();
    if !spec.requires_any.is_empty() && !spec.requires_any.iter().any(|c| declared.contains(c)) {
        missing.extend(spec.requires_any);
    }
    if !missing.is_empty() {
        return Some(CaseOutcome::Skipped { missing });
    }
    if !spec.requires_undeclared_any.is_empty()
        && spec
            .requires_undeclared_any
            .iter()
            .all(|c| declared.contains(c))
    {
        let names: Vec<String> = spec
            .requires_undeclared_any
            .iter()
            .map(|c| c.name())
            .collect();
        return Some(CaseOutcome::Inapplicable {
            reason: format!(
                "the subject declares every one of {}, so none is left to refuse",
                names.join(", ")
            ),
        });
    }
    None
}
