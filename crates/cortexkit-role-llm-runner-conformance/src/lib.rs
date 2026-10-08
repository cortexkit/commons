//! Conformance runner for the `llm-runner/v1` role.
//!
//! # Contract vocabulary
//!
//! Section references name the `CONTRACT.md` in `cortexkit-role-llm-runner`
//! or `cortexkit-role-compaction-provider`, as indicated below.
//!
//! - **Setup** is the compaction provider's initial session call. Its initial
//!   CompactionMessage must be durable before the first model request, and a
//!   recorded initial CompactionMessage prevents another Setup call
//!   (runner §11.1; compaction-provider §4).
//! - A **step** prepares one model request, either for a user turn or after
//!   tool results. When called, the compaction provider receives a status for
//!   that request (runner §11.1; compaction-provider §6).
//! - A **fence** is the per-request check that rejects an answer naming a
//!   request other than the newest, or a second view answer for an already
//!   accepted step. Passing the fence does not bypass the deadline or version
//!   checks (runner §11.1; compaction-provider §9).
//! - A **fold** applies a recorded CompactionMessage to the canonical model
//!   view before the affected model request is sent (runner §§8, 14).
//! - A **role code** is the compaction refusal's `code` (`RefuseCode`), which
//!   determines retryability. The provider's optional **finer code** is its
//!   diagnostic `provider_code`. Run errors name the role code `provider_code`
//!   and the finer code `provider_detail_code`; the finer code must not replace
//!   the role code or decide retryability (runner §§6, 11.1;
//!   compaction-provider §11).
//! - A **held model call** is a test mechanism: the scripted model captures a
//!   runner request but withholds its answer until explicitly released. It
//!   lets a case inspect the canonical input and durable records before the
//!   model answers. This is not an additional role operation (runner §§8,
//!   11.1 describe the view and persistence rules being checked).
//! - The **kill ledger** is the conformance driver's record of completed kills
//!   and their mechanisms. Its **kill points** name durable boundaries where
//!   the harness stops the runner. `CompactionApplied` leaves an answer durable
//!   before its fold; `FoldRecorded` leaves the prefix rebuild durable. At both
//!   points the affected model request has not been sent. The ledger must
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
//! Every place a case narrows a check because the role leaves the rule
//! open is listed in [`NARROWINGS`] and printed with the report.
//!
//! ```ignore
//! let report = run_suite(&my_runner_subject, tempdir.path()).await?;
//! println!("{}", report.render());
//! assert_eq!(report.verdict, SuiteVerdict::Passed);
//! ```

#![forbid(unsafe_code)]

mod cases;
mod compaction;
mod crash;
mod drive;
mod report;
mod retention;
mod route;
mod runner;
mod subject;

pub use compaction::{
    CompactionAnswer, CompactionCall, CompactionCallKind, CompactionMessage, CompactionObservation,
    CompactionScript, CompactionSetup, CompactionStep,
};
pub use cortexkit_role_harness as harness;
pub use cortexkit_role_llm_runner as wire;
pub use report::{CaseOutcome, CaseReport, CaseSpec, SuiteReport, SuiteVerdict, CASES, NARROWINGS};
pub use route::{envelope, Reply, RouteFailure, RunnerRoute, SubscribeOutcome};
pub use runner::{run_suite, SetupError};
pub use subject::{
    Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn,
};

/// The one tool every scripted call names. The runner adapter's scripted tool
/// provider serves it, and the session plan it builds makes it available.
pub const SCRIPTED_TOOL: &str = "conformance_echo";
