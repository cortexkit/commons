//! Conformance runner for the `llm-runner/v1` role.
//!
//! # Contract vocabulary
//!
//! Section references name the `CONTRACT.md` in `cortexkit-role-llm-runner`
//! or `cortexkit-role-compaction-provider`, as indicated below.
//!
//! - **Setup** is the compaction provider's call at the start of each
//!   lineage. Its initial CompactionMessage must be durable before the
//!   lineage's first model request, and a recorded initial CompactionMessage
//!   prevents another Setup call within the lineage (runner §11.1;
//!   compaction-provider §4).
//! - A **lineage** is the readable history from the session's latest cut; a
//!   cut, a revert or a recovery that drops or rewrites messages starts a new
//!   one (runner §4.5).
//! - A **fold** applies a recorded CompactionMessage to the canonical model
//!   view before the affected model request is sent (runner §§8, 14).
//! - A **role code** is the compaction refusal's `code` (`RefuseCode`), which
//!   determines retryability. The provider's optional **finer code** is its
//!   diagnostic `provider_code`. Run errors name the role code `provider_code`
//!   and the finer code `provider_detail_code`; the finer code must not replace
//!   the role code or decide retryability (runner §§6, 11.1;
//!   compaction-provider §10).
//! - A **held model call** is a test mechanism: the scripted model captures a
//!   runner request but withholds its answer until explicitly released. It
//!   lets a case inspect the canonical input and durable records before the
//!   model answers. This is not an additional role operation (runner §§8,
//!   11.1 describe the view and persistence rules being checked).
//! - The **kill ledger** is the conformance driver's record of completed kills
//!   and their mechanisms. Its **kill points** name durable boundaries where
//!   the harness stops the runner. `FoldRecorded` leaves Setup's prefix
//!   rebuild durable before the affected model request is sent. The ledger must
//!   contain at least one real process kill (runner §§14, 15).
//!
//! # Runner adapter
//!
//! The runner adapter implements [`LlmRunnerSubject`] and its harness. The
//! conformance driver is [`run_suite`], which runs the cases in [`CASES`]. Run
//! conformance in the runner's own CI against its real module and management
//! route, never against a double. The adapter supplies:
//!
//! - a [`cortexkit_role_harness::Harness`] that spawns the runner on a state
//!   root, kills it at the role's named points
//!   (`cortexkit_role_llm_runner::points`) and restarts it on the same root;
//! - a [`RunnerRoute`] adapter for its route type, which sends one role
//!   request `{method, params}` over the real route and reports the answer;
//! - the [`LlmRunnerSubject`] facts the conformance driver cannot discover:
//!   the capability groups and harness capabilities it declares, how to open a session's
//!   route, the fields a session's first send needs, and a scripted model
//!   and tool/compaction providers. Cases provide each session's model as a
//!   [`Script`] of assistant turns (text, reasoning and tool-call parts,
//!   with each call's scripted result). Cases do not speak a model provider's
//!   wire protocol; the runner adapter backs the script with a mock
//!   provider that speaks the runner's model protocol.
//!
//! [`run_suite`] then runs every case in [`CASES`]. A case whose
//! requirements the runner adapter does not declare is reported as skipped,
//! naming the missing capabilities, never as passed. The verdict is a plain pass only
//! when every case ran and passed; otherwise, with nothing failed, it is
//! "conforming for declared capabilities", naming them. The run fails if any
//! case fails, or if no kill ended a real process: simulated kills cannot
//! catch a runner that keeps something only in memory.
//!
//! [`run_suite_with`] takes [`SuiteOptions`]; its progress callback hears
//! each case start and end ([`SuiteProgress`]), and
//! [`progress::stderr_lines`] prints those events to stderr, so a run cut
//! off before its report still names the case it was in.
//!
//! Every place a case narrows a check because the role leaves the rule
//! open is listed in [`NARROWINGS`] and printed with the report.
//!
//! ```ignore
//! let report = run_suite(&my_runner_subject, tempdir.path()).await?;
//! println!("{}", report.render());
//! assert_eq!(report.verdict, SuiteVerdict::Passed);
//! ```

#![forbid(unsafe_code)]

mod adopt;
mod archive_pruning;
mod attachments;
mod cache_ttl;
pub mod call_identity;
mod cancel;
mod cases;
mod ceiling;
mod compaction;
mod cost;
mod crash;
mod delivery_order;
mod directives;
mod display;
mod drive;
mod follow;
mod head_sender;
mod held_calls;
mod host_history;
mod incarnation;
mod input_token_cap;
mod interrupt_reason;
mod last_run_error;
mod lineage;
mod live;
mod media;
pub mod message_checkpoint;
mod output_refs;
pub mod progress;
mod report;
mod retention;
pub mod rich_ui_stamp;
mod ride_wait;
mod route;
mod run_parameters;
mod run_usage;
mod runner;
mod session_change;
mod session_retention;
mod session_usage;
pub mod step_events;
mod structured_output;
mod subject;
mod system_text_default;
mod tags;
mod tool_calls;
mod user_commands;

pub use compaction::{
    CompactionCall, CompactionMessage, CompactionObservation, CompactionRouteIdentity,
    CompactionSetup, CompactionSetupIdentity, ProviderCall, ScriptedPartEdit, ScriptedRange,
    ScriptedRide, ScriptedView,
};
pub use cortexkit_role_harness as harness;
pub use cortexkit_role_llm_runner as wire;
pub use progress::{SuiteOptions, SuiteProgress};
pub use report::{CaseOutcome, CaseReport, CaseSpec, SuiteReport, SuiteVerdict, CASES, NARROWINGS};
pub use route::{envelope, Reply, RouteFailure, RunnerRoute, SubscribeOutcome};
pub use runner::{run_suite, run_suite_with, SetupError};
pub use subject::{
    ArchivePruningPart, AttributionFixture, Capability, LlmRunnerSubject, ModelRequestObservation,
    PlanSenderIdentity, RefreshPlan, RefreshVariant, Script, ScriptedLateEntry, ScriptedPart,
    ScriptedToolCall, ScriptedTurn, ScriptedUsage, SystemTextDefaultObservation,
    ToolResultObservation, UserCommandFixture, UserCommandObservation,
};

/// The one tool every scripted call names. The runner adapter's scripted tool
/// provider serves it, and the session plan it builds makes it available.
pub const SCRIPTED_TOOL: &str = "conformance_echo";
