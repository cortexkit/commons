use crate::{cases, AgentInboxSubject, CaseOutcome, CaseReport, SuiteReport, SuiteVerdict, CASES};

/// Run every case in isolation, collecting failures rather than failing fast.
pub fn run(subject: &mut impl AgentInboxSubject) -> SuiteReport {
    let mut reports = Vec::new();
    let mut reasons = Vec::new();
    for spec in CASES {
        let outcome = match subject
            .reset(spec.name)
            .and_then(|()| cases::run(subject, spec.name))
        {
            Ok(()) => CaseOutcome::Passed,
            Err(reason) => {
                reasons.push(format!("{}: {reason}", spec.name));
                CaseOutcome::Failed { reason }
            }
        };
        reports.push(CaseReport {
            case: spec.name,
            outcome,
        });
    }
    let verdict = if reasons.is_empty() {
        SuiteVerdict::Passed
    } else {
        SuiteVerdict::Failed { reasons }
    };
    SuiteReport {
        cases: reports,
        verdict,
    }
}
