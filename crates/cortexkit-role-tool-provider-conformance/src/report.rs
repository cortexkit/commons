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

use Capability::{
    ApprovalExecution, CallKey, Cancellation, DisableTool, HeldCalls, LateResults, SchemaPin,
    ScopeStamp,
};

const WITHDRAW: &[Capability] = &[HeldCalls, ScopeStamp, CallKey];
const LATE: &[Capability] = &[
    LateResults,
    ScopeStamp,
    HeldCalls,
    CallKey,
    ApprovalExecution,
];
const CRASH: &[Capability] = &[
    ApprovalExecution,
    HeldCalls,
    ScopeStamp,
    CallKey,
    LateResults,
];

/// Every case, in the order the runner runs them.
pub const CASES: &[CaseSpec] = &[
    CaseSpec {
        name: "role_describe_shape",
        requires: &[],
        checks: "role.describe lists v1 among its versions and every required op, carries no tool list, and lists tool.withdraw or the late_results ops when the subject declares held_calls or late_results",
    },
    CaseSpec {
        name: "role_describe_cacheable",
        requires: &[],
        checks: "two role.describe answers from one module are identical",
    },
    CaseSpec {
        name: "catalog_schemas_flat",
        requires: &[],
        checks: "every catalog tool has a schema_digest and a flat input_schema, no role op is listed as a tool, and the quick call's tool is listed",
    },
    CaseSpec {
        name: "catalog_schema_digest_stable",
        requires: &[],
        checks: "two tool.catalog answers carry the same generation and the same tools with the same schema_digest and semantics",
    },
    CaseSpec {
        name: "catalog_digest_only",
        requires: &[],
        checks: "a digest_only answer carries the full answer's generation and a catalog_digest, and no tools",
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
        checks: "a call to a tool the provider does not serve gets exactly one terminal frame, an unknown_tool error",
    },
    CaseSpec {
        name: "terminal_frame_on_cancel",
        requires: &[Cancellation],
        checks: "a cancelled call gets exactly one terminal frame",
    },
    CaseSpec {
        name: "call_key_malformed_refused",
        requires: &[CallKey],
        checks: "every invalid call_key vector is refused as invalid_request {field: \"call_key\"}",
    },
    CaseSpec {
        name: "call_key_well_formed_accepted",
        requires: &[CallKey],
        checks: "every valid call_key vector is accepted",
    },
    CaseSpec {
        name: "schema_pin_current_accepted",
        requires: &[SchemaPin],
        checks: "a call pinned to its tool's current generation and semantics is accepted",
    },
    CaseSpec {
        name: "schema_pin_malformed_refused",
        requires: &[SchemaPin],
        checks: "every refused schema-pin vector, and a pin naming another tool, is refused as invalid_request {field: \"schema_pin\"}",
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
        name: "withdraw_owner_as_carrier_needs_no_carrier",
        requires: WITHDRAW,
        checks: "the scope's owner withdrawing a call it raised itself, without arguments.carrier, is treated as the carrier and gets withdrawn",
    },
    CaseSpec {
        name: "withdraw_owner_without_carrier_refused",
        requires: WITHDRAW,
        checks: "the scope's owner withdrawing another carrier's call without arguments.carrier gets the route error withdraw_carrier_required",
    },
    CaseSpec {
        name: "withdraw_carrier_naming_other_carrier_refused",
        requires: WITHDRAW,
        checks: "a carrier naming a different carrier gets the route error withdraw_carrier_mismatch",
    },
    CaseSpec {
        name: "withdraw_scope_mismatch_refused",
        requires: WITHDRAW,
        checks: "arguments.scope differing from the route's stamp gets the route error withdraw_scope_mismatch",
    },
    CaseSpec {
        name: "withdraw_top_level_call_key_refused",
        requires: WITHDRAW,
        checks: "a tool.withdraw request with its own top-level call_key is refused as invalid_request {field: \"call_key\"}",
    },
    CaseSpec {
        name: "withdraw_malformed_target_key_refused",
        requires: WITHDRAW,
        checks: "a malformed arguments.call_key is refused as invalid_request {field: \"arguments.call_key\"}",
    },
    CaseSpec {
        name: "withdraw_not_permitted_is_route_error",
        requires: WITHDRAW,
        checks: "the carrier withdrawing its call from a route under another scope gets the route error withdraw_not_permitted",
    },
    CaseSpec {
        name: "late_results_cursor_round_trip",
        requires: LATE,
        checks: "the custodian reads a settled call's result entry from null, and reading on from the returned cursor yields nothing already read, identically twice",
    },
    CaseSpec {
        name: "late_results_ack",
        requires: LATE,
        checks: "after late_results.ack through a cursor, a read from null no longer returns the acked entry",
    },
    CaseSpec {
        name: "late_results_incarnation_change_refused",
        requires: &[LateResults, ScopeStamp],
        checks: "the catalog declares late_results, and a cursor from another incarnation or past the log is refused as cursor_incarnation_changed",
    },
    CaseSpec {
        name: "crash_after_prepared_not_started",
        requires: CRASH,
        checks: "killed at Prepared and restarted, the provider never runs the call, reports a not_started late result for it, and its withdraw answer guarantees it never runs",
    },
    CaseSpec {
        name: "crash_after_authorized_not_started",
        requires: CRASH,
        checks: "killed at Authorized, before DispatchStarted, and restarted, the provider never runs the call, reports a not_started late result for it, and its withdraw answer guarantees it never runs",
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
    /// Nothing failed, but the subject does not declare every capability, so
    /// the cases requiring `skipped` were not run. The provider conforms for
    /// the capabilities it declares, and for no others. Never a plain pass.
    ConformingForDeclaredCapabilities {
        skipped: Vec<Capability>,
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
