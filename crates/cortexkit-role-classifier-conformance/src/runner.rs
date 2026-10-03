//! Running the checks and computing the verdict.

use crate::{
    checks::{Ending, Suite},
    report::{CheckOutcome, CheckReport, SuiteReport, SuiteVerdict, CHECKS},
    route::ClassifierRoute,
    subject::ClassifierSubject,
};

/// Run every check against `subject`.
pub async fn run_suite<S>(subject: &S) -> SuiteReport
where
    S: ClassifierSubject,
    S::Route: ClassifierRoute,
{
    let suite = Suite::open(subject).await;
    let mut checks = Vec::with_capacity(CHECKS.len());
    for spec in CHECKS {
        let outcome = match &suite {
            Err(reason) => CheckOutcome::Failed {
                reason: reason.clone(),
            },
            Ok(suite) => match suite.run(spec.name).await {
                Ok(Ending::Passed) => CheckOutcome::Passed,
                Ok(Ending::NotApplicable(reason)) => CheckOutcome::NotApplicable { reason },
                Err(reason) => CheckOutcome::Failed { reason },
            },
        };
        checks.push(CheckReport {
            check: spec.name,
            outcome,
        });
    }
    let failures: Vec<String> = checks
        .iter()
        .filter_map(|report| match &report.outcome {
            CheckOutcome::Failed { reason } => Some(format!("{}: {reason}", report.check)),
            _ => None,
        })
        .collect();
    let not_applicable: Vec<&'static str> = checks
        .iter()
        .filter(|report| matches!(report.outcome, CheckOutcome::NotApplicable { .. }))
        .map(|report| report.check)
        .collect();
    let verdict = if !failures.is_empty() {
        SuiteVerdict::Failed { reasons: failures }
    } else if !not_applicable.is_empty() {
        SuiteVerdict::PassedExceptNotApplicable { not_applicable }
    } else {
        SuiteVerdict::Passed
    };
    SuiteReport { checks, verdict }
}
