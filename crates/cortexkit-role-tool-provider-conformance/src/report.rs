//! The suite's cases and what a run reports.

use cortexkit_role_harness::KillReport;

use crate::subject::Capability;

/// One conformance case: its stable name, what it requires, and what it
/// checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaseSpec {
    pub name: &'static str,
    pub requires: &'static [Capability],
    pub checks: &'static str,
}

use Capability::{ApprovalExecution, Cancellation, DisableTool, HeldCalls, ScopeStamp};

const WITHDRAW: &[Capability] = &[HeldCalls, ScopeStamp, Capability::CallKey];
const CRASH: &[Capability] = &[
    ApprovalExecution,
    HeldCalls,
    ScopeStamp,
    Capability::CallKey,
];

/// Every case, in the order the runner runs them.
pub const CASES: &[CaseSpec] = &[
    CaseSpec {
        name: "role_describe_shape",
        requires: &[],
        checks: "role.describe answers tool-provider/v1 with every required op, carries no tool list, and lists tool.withdraw when the subject declares held_calls",
    },
    CaseSpec {
        name: "role_describe_cacheable",
        requires: &[],
        checks: "two role.describe answers from one module are identical",
    },
    CaseSpec {
        name: "catalog_schemas_flat",
        requires: &[],
        checks: "every catalog tool has a schema_digest and a flat input_schema, and the quick call's tool is listed",
    },
    CaseSpec {
        name: "catalog_schema_digest_stable",
        requires: &[],
        checks: "two tool.catalog answers list the same tools with the same schema_digest",
    },
    CaseSpec {
        name: "catalog_disabled_tool_absent",
        requires: &[DisableTool],
        checks: "a tool disabled by its exact name is absent from tool.catalog",
    },
    CaseSpec {
        name: "call_disabled_tool_refused_by_name",
        requires: &[DisableTool],
        checks: "a call to the disabled tool is refused as tool_disabled {tool}",
    },
    CaseSpec {
        name: "terminal_frame_on_success",
        requires: &[],
        checks: "a successful call gets exactly one terminal frame, not an error",
    },
    CaseSpec {
        name: "terminal_frame_on_refusal",
        requires: &[],
        checks: "a call to a tool the provider does not serve gets exactly one terminal frame, an error",
    },
    CaseSpec {
        name: "terminal_frame_on_cancel",
        requires: &[Cancellation],
        checks: "a cancelled call gets exactly one terminal frame",
    },
    CaseSpec {
        name: "call_key_malformed_refused",
        requires: &[Capability::CallKey],
        checks: "every invalid call_key vector is refused as invalid_request {field: \"call_key\"}",
    },
    CaseSpec {
        name: "call_key_well_formed_accepted",
        requires: &[Capability::CallKey],
        checks: "every valid call_key vector is accepted (not refused naming call_key)",
    },
    CaseSpec {
        name: "withdraw_unknown_call",
        requires: WITHDRAW,
        checks: "withdrawing a key the provider never held answers unknown_call, never a refusal",
    },
    CaseSpec {
        name: "withdraw_answer_identical_on_repeat_and_owner",
        requires: WITHDRAW,
        checks: "a held call's carrier gets withdrawn, the same bytes on repeat and for the owner naming the carrier, and the action never runs",
    },
    CaseSpec {
        name: "withdraw_owner_without_carrier_refused",
        requires: WITHDRAW,
        checks: "the scope's owner withdrawing without arguments.carrier is refused with an error frame, not answered",
    },
    CaseSpec {
        name: "withdraw_carrier_naming_other_carrier_refused",
        requires: WITHDRAW,
        checks: "a carrier naming a different carrier is refused with an error frame, not answered",
    },
    CaseSpec {
        name: "withdraw_scope_mismatch_refused",
        requires: WITHDRAW,
        checks: "arguments.scope differing from the route's stamp is refused with an error frame",
    },
    CaseSpec {
        name: "withdraw_top_level_call_key_refused",
        requires: WITHDRAW,
        checks: "a tool.withdraw request with its own top-level call_key is refused as invalid_request {field: \"call_key\"}",
    },
    CaseSpec {
        name: "withdraw_not_permitted_is_route_error",
        requires: WITHDRAW,
        checks: "a caller that is neither the carrier nor the owner gets the route error withdraw_not_permitted",
    },
    CaseSpec {
        name: "crash_after_prepared_not_started",
        requires: CRASH,
        checks: "killed at Prepared and restarted, the provider never runs the call and its withdraw answer guarantees it never will",
    },
    CaseSpec {
        name: "crash_after_authorized_not_started",
        requires: CRASH,
        checks: "killed at Authorized, before DispatchStarted, and restarted, the provider never runs the call and its withdraw answer guarantees it never will",
    },
];

/// How one case ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaseOutcome {
    Passed,
    Failed {
        reason: String,
    },
    /// Not run: the subject does not declare these capabilities.
    Skipped {
        missing: Vec<Capability>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseReport {
    pub case: &'static str,
    pub requires: Vec<Capability>,
    pub outcome: CaseOutcome,
}

/// The run's overall result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SuiteVerdict {
    /// Every case ran and passed, and at least one kill ended a real process.
    Passed,
    /// Nothing failed, but some cases were skipped. Not a pass.
    Incomplete {
        skipped: Vec<&'static str>,
    },
    Failed {
        reasons: Vec<String>,
    },
}

/// Everything a run found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SuiteReport {
    pub cases: Vec<CaseReport>,
    /// Every kill the run made, with the mechanism the harness reported.
    pub kills: Vec<KillReport>,
    pub verdict: SuiteVerdict,
}

impl SuiteReport {
    pub fn outcome(&self, case: &str) -> Option<&CaseOutcome> {
        self.cases
            .iter()
            .find(|report| report.case == case)
            .map(|report| &report.outcome)
    }

    /// A human-readable summary, one line per case, for a CI log.
    pub fn render(&self) -> String {
        let mut out = String::new();
        for case in &self.cases {
            let requires: Vec<&str> = case.requires.iter().map(|c| c.name()).collect();
            let line = match &case.outcome {
                CaseOutcome::Passed => format!("PASS {}", case.case),
                CaseOutcome::Failed { reason } => format!("FAIL {}: {reason}", case.case),
                CaseOutcome::Skipped { missing } => {
                    let missing: Vec<&str> = missing.iter().map(|c| c.name()).collect();
                    format!(
                        "SKIP {} (requires {}; missing {})",
                        case.case,
                        requires.join(", "),
                        missing.join(", ")
                    )
                }
            };
            out.push_str(&line);
            out.push('\n');
        }
        for kill in &self.kills {
            out.push_str(&format!("KILL at {} by {}\n", kill.point, kill.mechanism));
        }
        out.push_str(&format!("VERDICT {:?}\n", self.verdict));
        out
    }
}
