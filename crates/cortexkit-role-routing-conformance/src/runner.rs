//! Running the checks and computing the verdict.

use crate::{
    checks::{Ending, Suite},
    report::{CheckOutcome, CheckReport, SuiteReport, SuiteVerdict, CHECKS},
    subject::RoutingSubject,
};

/// Run every check against `subject`, each from a freshly reset and
/// configured provider.
pub async fn run_suite<S: RoutingSubject>(subject: &S) -> SuiteReport {
    let suite = Suite::new(subject);
    let mut checks = Vec::with_capacity(CHECKS.len());
    for spec in CHECKS {
        let outcome = match suite.run(spec.name).await {
            Ok(Ending::Passed) => CheckOutcome::Passed,
            Ok(Ending::NotApplicable(reason)) => CheckOutcome::NotApplicable { reason },
            Err(reason) => CheckOutcome::Failed { reason },
        };
        checks.push(CheckReport {
            check: spec.name,
            outcome,
        });
    }
    let reasons: Vec<String> = checks
        .iter()
        .filter_map(|report| match &report.outcome {
            CheckOutcome::Failed { reason } => Some(format!("{}: {reason}", report.check)),
            _ => None,
        })
        .collect();
    let verdict = if reasons.is_empty() {
        SuiteVerdict::Passed
    } else {
        SuiteVerdict::Failed { reasons }
    };
    SuiteReport { checks, verdict }
}
