//! Stable case names and an explicit report; no unchecked case is a pass.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaseSpec {
    pub name: &'static str,
    pub checks: &'static str,
}

macro_rules! cases {
    ($($name:literal => $checks:literal),+ $(,)?) => {
        pub const CASES: &[CaseSpec] = &[$(CaseSpec { name: $name, checks: $checks }),+];
    };
}

cases! {
    "repeat_first_reply" => "same body returns first durable acceptance, without another item, including after restart",
    "conflicting_repeat" => "same key with different body refuses idempotency_conflict and retains first receipt",
    "retry_before_authorization" => "accepted retries and conflicts resolve before fresh authorization after restart and scope end",
    "replace_live_attention_and_timing" => "live replacement keeps strongest attention and newest not-before and expiry",
    "replace_delivered_untouched" => "replacement never mutates or removes a landed edition",
    "replace_retired_attention" => "expired and withdrawn editions do not lend their wake attention",
    "expiry_drops" => "expired pending editions never reach a receiver",
    "deliver_after_holds" => "not-before holds until its absolute millisecond deadline",
    "withdraw_withdrawn" => "both key selectors withdraw pending items, with no later delivery",
    "withdraw_already_delivered" => "landed items answer already_delivered with both selectors",
    "withdraw_not_found" => "missing and already-withdrawn keys answer not_found",
    "namespace_isolation" => "principal, delegated source and target isolate retries, replacements and withdrawals",
    "scope_not_yours" => "daemon not_served maps to scope_not_yours",
    "scope_unknown" => "daemon unknown maps to scope_unknown",
    "scope_ended" => "ended scope maps to scope_ended",
    "neutralization_delivered" => "per-item user-authority envelope neutralizes text, slots, labels, URLs and rendered data",
    "content_cap" => "whole serialized notice accepts at 16 KiB and refuses above it, including data",
    "pull_recovers" => "held and unconfirmed bodies remain available through own-inbox list and read",
    "inbox_full_retry" => "inbox_full is retryable with the same key after capacity becomes free",
}

pub const NARROWINGS: &[&str] = &[
    "fixture hooks control daemon served history, clock, scheduler boundaries, quota and one ambiguous delivery; adapters must not synthesize operation replies or receiver observations",
    "restart checks durable receipts, not crash atomicity, real process kill points or receipt retention over the sender's full retry horizon",
    "flow namespace cases require stamped carrier routes and a wake grant; live flow grant/owner/audience and template-allowlist refusal policy are covered by vectors, not live fixtures",
    "scheduler policy, DND, residence fencing and runner reconciliation beyond the unconfirmed fixture are outside this wire-role suite",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseOutcome {
    Passed,
    Failed { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseReport {
    pub case: &'static str,
    pub outcome: CaseOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuiteVerdict {
    Passed,
    Failed { reasons: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteReport {
    pub cases: Vec<CaseReport>,
    pub verdict: SuiteVerdict,
}

impl SuiteReport {
    pub fn outcome(&self, case: &str) -> Option<&CaseOutcome> {
        self.cases
            .iter()
            .find(|report| report.case == case)
            .map(|report| &report.outcome)
    }
    pub fn render(&self) -> String {
        let mut out = String::new();
        for case in &self.cases {
            out.push_str(&match &case.outcome {
                CaseOutcome::Passed => format!("PASS {}\n", case.case),
                CaseOutcome::Failed { reason } => format!("FAIL {}: {reason}\n", case.case),
            });
        }
        for narrowing in NARROWINGS {
            out.push_str(&format!("NOTE {narrowing}\n"));
        }
        out.push_str(&format!("VERDICT {:?}\n", self.verdict));
        out
    }
}
