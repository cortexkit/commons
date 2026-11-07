//! Live conformance for `llm-runner/v1`, pinned to the runner 0.8 contract.
//!
//! Implement [`LlmRunnerSubject`] against a real management route and scripted
//! model/tool endpoints, then call [`run_suite`] with an empty work directory.
//! [`CASES`] gives every named check and its capability gates. The in-process
//! fake is used only by this kit's own tests, including deliberately incorrect
//! runners that must be rejected by name.
//!
//! Compaction-provider and compaction-runner protocol checks belong outside
//! this kit. Native marker hooks test only the runner's full-path read and
//! rewrite-identity rules. See the README for adapter and independent-observation
//! requirements, and [`NARROWINGS`] for limits of executable coverage.

#![forbid(unsafe_code)]

#[cfg(test)]
extern crate self as cortexkit_role_llm_runner_conformance;
#[cfg(test)]
#[allow(dead_code, unreachable_patterns)]
#[path = "../tests/fake/mod.rs"]
mod fork_fake;

mod account_route;
mod admission;
mod adopt;
mod archive_pruning;
mod attachments;
mod attribution;
pub mod call_identity;
mod cancel;
mod cases;
mod cost;
mod crash;
mod delivery_order;
mod directives;
mod display;
mod drive;
mod finish_reason;
mod follow;
mod fork_answer;
mod head_selection;
mod head_sender;
mod held_calls;
mod history_intake;
mod host_history;
mod input_token_cap;
mod interrupt_reason;
mod last_run_error;
mod live;
mod live_ids;
mod media;
mod message_marks;
mod message_senders;
mod notices;
mod output_refs;
mod plan_notes;
pub mod progress;
mod read_contract;
mod read_display_facts;
mod report;
mod retention;
pub mod rich_ui_stamp;
mod route;
pub mod routed_accounts;
mod run_parameters;
mod run_usage;
mod runner;
mod scope_admission;
mod send_helpers;
mod sender_succession;
mod service_tier;
mod session_change;
mod session_retention;
mod session_usage;
pub mod step_events;
mod structured_output;
mod subject;
mod system_text_default;
mod tool_calls;
mod tool_result_limit;
pub mod upfront;
mod user_commands;

pub use cortexkit_role_harness as harness;
pub use cortexkit_role_llm_runner as wire;
pub use progress::{SuiteOptions, SuiteProgress};
pub use report::{CaseOutcome, CaseReport, CaseSpec, SuiteReport, SuiteVerdict, CASES, NARROWINGS};
pub use route::{envelope, Reply, RouteFailure, RunnerRoute, SubscribeOutcome};
pub use routed_accounts::RecordingRoutingPeer;
pub use runner::{run_suite, run_suite_with, SetupError};
pub use subject::{
    ArchivePruningPart, AttributionFixture, Capability, LiveIdsObservation, LlmRunnerSubject,
    ModelRequestObservation, PlanNotesFixture, PlanSenderIdentity, RawModelRequestObservation,
    RefreshPlan, RefreshVariant, Script, ScriptedLateEntry, ScriptedPart, ScriptedToolCall,
    ScriptedTurn, ScriptedUsage, SystemTextDefaultObservation, ToolResultLimitObservation,
    ToolResultObservation, UserCommandFixture, UserCommandObservation,
};

/// The one tool every scripted call names. The runner adapter's scripted tool
/// provider serves it, and the session plan it builds makes it available.
///
/// Its input schema is [`call_identity::DISPATCHED_DEFINITION`]'s: an object
/// whose `marker` is a required string, with other members allowed. Every
/// scripted call to it carries a string `marker`, so its input passes that
/// schema's validation on a host that checks input before running a call.
pub const SCRIPTED_TOOL: &str = "conformance_echo";
