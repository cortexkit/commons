mod logs;
mod project;

use std::{borrow::Cow, fmt, path::PathBuf};

use serde::Serialize;

use crate::{
    redact_secrets, CheckOutput, Crash, FindingId, FixClass, FixOutput, FixResult, Manifest,
    ModuleId, RunError, Severity,
};

use self::{
    logs::Log,
    project::{ProjectedEvidence, Projection},
};

/// The key, reference time and path roots used to sanitize one report.
pub struct ReportContext {
    report_key: [u8; 32],
    report_time_ms: i64,
    data_home: PathBuf,
    config_home: PathBuf,
    temp_dir: PathBuf,
}

impl ReportContext {
    /// Supply a fresh random key for each report. The reference time is in
    /// milliseconds since the Unix epoch. Paths are compared lexically.
    pub fn new(
        report_key: [u8; 32],
        report_time_ms: i64,
        data_home: impl Into<PathBuf>,
        config_home: impl Into<PathBuf>,
        temp_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            report_key,
            report_time_ms,
            data_home: data_home.into(),
            config_home: config_home.into(),
            temp_dir: temp_dir.into(),
        }
    }
}

/// Validated outputs and raw log tails for one module's report.
#[non_exhaustive]
pub struct ReportInput<'a> {
    pub module_id: &'a ModuleId,
    pub manifest: &'a Manifest,
    pub title: &'a str,
    pub description: &'a str,
    pub check: Result<&'a CheckOutput, RunError>,
    pub max_body_bytes: usize,
    pub fix_results: &'a [(FindingId, Result<FixOutput, RunError>)],
    pub log_tails: &'a [&'a str],
}

impl<'a> ReportInput<'a> {
    /// Start with no fix results and no log tails.
    pub fn new(
        module_id: &'a ModuleId,
        manifest: &'a Manifest,
        title: &'a str,
        description: &'a str,
        check: Result<&'a CheckOutput, RunError>,
        max_body_bytes: usize,
    ) -> Self {
        Self {
            module_id,
            manifest,
            title,
            description,
            check,
            max_body_bytes,
            fix_results: &[],
            log_tails: &[],
        }
    }

    /// Include fix results in caller order.
    pub fn with_fix_results(
        mut self,
        results: &'a [(FindingId, Result<FixOutput, RunError>)],
    ) -> Self {
        self.fix_results = results;
        self
    }

    /// Include raw log tails; caller order breaks ties between timestamps.
    pub fn with_log_tails(mut self, log_tails: &'a [&'a str]) -> Self {
        self.log_tails = log_tails;
        self
    }
}

/// A report refused without emitting any of its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportError {
    /// The manifest belongs to another module.
    ModuleMismatch,
    /// The report does not fit even without logs.
    TooLarge,
    /// All eight windows for a path or ref are already in use.
    AliasExhausted,
}

impl ReportError {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ModuleMismatch => "module_mismatch",
            Self::TooLarge => "too_large",
            Self::AliasExhausted => "alias_exhausted",
        }
    }
}

impl fmt::Display for ReportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::error::Error for ReportError {}

/// Build RFC 8785 canonical JSON with no trailing newline. Only log lines can
/// be evicted to meet the byte budget. Review free text before publishing the
/// report: the secret pass does not make arbitrary text safe to publish.
pub fn build_report(ctx: &ReportContext, input: &ReportInput<'_>) -> Result<Vec<u8>, ReportError> {
    if input.module_id != input.manifest.module_id() {
        return Err(ReportError::ModuleMismatch);
    }
    let mut projection = Projection::new(ctx);
    let (check, findings) = match input.check {
        Ok(output) => {
            let findings = output
                .findings()
                .iter()
                .map(|finding| {
                    let fix = finding.fix();
                    let steps = fix
                        .plan()
                        .map(|plan| {
                            plan.steps()
                                .iter()
                                .map(|step| redact_secrets(step))
                                .collect()
                        })
                        .unwrap_or_default();
                    Ok(ReportFinding {
                        id: finding.id().as_str(),
                        severity: finding.severity(),
                        summary: redact_secrets(finding.summary()),
                        evidence: projection.evidence(finding.evidence())?,
                        fix: ReportFix {
                            class: fix.class(),
                            needs_stop: fix.needs_stop(),
                            steps,
                        },
                    })
                })
                .collect::<Result<_, ReportError>>()?;
            (Status::new("ok"), findings)
        }
        Err(failure) => (Status::failure(failure), Vec::new()),
    };
    let fix_results = input
        .fix_results
        .iter()
        .map(|(id, result)| {
            let (status, evidence) = match result {
                Ok(output) => {
                    let mut status = Status::new(match output.result() {
                        FixResult::Fixed => "fixed",
                        FixResult::Refused => "refused",
                        FixResult::Failed => "failed",
                    });
                    status.code = output.code();
                    (
                        status,
                        projection.evidence(output.evidence().unwrap_or_default())?,
                    )
                }
                Err(failure) => (Status::failure(*failure), Vec::new()),
            };
            Ok(ReportFixResult {
                finding_id: id.as_str(),
                status,
                evidence,
            })
        })
        .collect::<Result<_, ReportError>>()?;
    let (logs, omitted_lines) = logs::sanitize(input.manifest, input.log_tails, &mut projection)?;
    encode_with_limit(
        Report {
            schema: "ck.doctor-report/v1",
            module_id: input.module_id.as_str(),
            repository: input.manifest.repository(),
            title: redact_secrets(input.title),
            description: redact_secrets(input.description),
            check,
            findings,
            fix_results,
            logs: &logs,
            dropped_values: projection.dropped_values,
            omitted_lines,
            evicted_lines: 0,
        },
        input.max_body_bytes,
    )
}

#[derive(Serialize)]
struct Report<'a> {
    schema: &'static str,
    module_id: &'a str,
    repository: &'a str,
    title: Cow<'a, str>,
    description: Cow<'a, str>,
    check: Status<'a>,
    findings: Vec<ReportFinding<'a>>,
    fix_results: Vec<ReportFixResult<'a>>,
    logs: &'a [Log<'a>],
    dropped_values: usize,
    omitted_lines: usize,
    evicted_lines: usize,
}

#[derive(Serialize)]
struct ReportFinding<'a> {
    id: &'a str,
    severity: Severity,
    summary: Cow<'a, str>,
    evidence: Vec<ProjectedEvidence<'a>>,
    fix: ReportFix<'a>,
}

#[derive(Serialize)]
struct ReportFix<'a> {
    class: FixClass,
    needs_stop: bool,
    steps: Vec<Cow<'a, str>>,
}

#[derive(Serialize)]
struct ReportFixResult<'a> {
    finding_id: &'a str,
    #[serde(flatten)]
    status: Status<'a>,
    evidence: Vec<ProjectedEvidence<'a>>,
}

#[derive(Serialize)]
struct Status<'a> {
    outcome: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    signal: Option<i32>,
}

impl Status<'_> {
    fn new(outcome: &'static str) -> Self {
        Self {
            outcome,
            code: None,
            exit_code: None,
            signal: None,
        }
    }

    fn failure(failure: RunError) -> Self {
        let mut status = Self::new(failure.as_str());
        match failure {
            RunError::InvalidOutput(reason) => status.code = Some(reason.as_str()),
            RunError::Crashed(Crash::ExitCode(exit)) => status.exit_code = Some(exit),
            RunError::Crashed(Crash::Signal(signal)) => status.signal = Some(signal),
            _ => {}
        }
        status
    }
}

fn encode(value: &impl Serialize) -> Vec<u8> {
    // Report values contain only string keys and validated finite numbers.
    serde_jcs::to_vec(value).expect("report values are canonical JSON")
}

fn encode_with_limit(mut report: Report<'_>, max_bytes: usize) -> Result<Vec<u8>, ReportError> {
    let full = encode(&report);
    let mut bytes = full.len();
    if bytes <= max_bytes {
        return Ok(full);
    }
    drop(full);
    let logs = report.logs;
    for (i, log) in logs.iter().enumerate() {
        let evicted = i + 1;
        // In the canonical array, each removed line also removes one comma
        // except when it was the last line. Count new digits above the initial 0.
        bytes -= encode(log).len() + usize::from(evicted < logs.len());
        if bytes + evicted.ilog10() as usize <= max_bytes {
            report.logs = &logs[evicted..];
            report.evicted_lines = evicted;
            return Ok(encode(&report));
        }
    }
    Err(ReportError::TooLarge)
}
