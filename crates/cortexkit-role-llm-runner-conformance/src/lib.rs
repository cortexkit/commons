//! Conformance runner for the `llm-runner/v1` role.
//!
//! A runner runs this suite in its own CI against its real module over its
//! real management route, never against a double. It supplies:
//!
//! - a [`cortexkit_role_harness::Harness`] that spawns the runner on a state
//!   root, kills it at the role's named points
//!   (`cortexkit_role_llm_runner::points`) and restarts it on the same root;
//! - a [`RunnerRoute`] adapter for its route type, which sends one role
//!   request `{method, params}` over the real route and reports the answer;
//! - the [`LlmRunnerSubject`] facts the suite cannot know: the capability
//!   groups and harness capabilities it declares, how to open a session's
//!   route, the fields a session's first send needs, and a scripted model
//!   and tool provider. The suite writes each session's model as a
//!   [`Script`] of assistant turns (text, reasoning and tool-call parts,
//!   with each call's scripted result) and never speaks a model provider's
//!   wire protocol itself; the subject backs the script with its own mock.
//!
//! [`run_suite`] then runs every case in [`CASES`]. A case whose
//! requirements the subject does not declare is reported as skipped with the
//! missing capabilities, never as passed. The verdict is a plain pass only
//! when every case ran and passed; otherwise, with nothing failed, it is
//! "conforming for declared capabilities", naming them. The run fails if any
//! case fails, or if no kill ended a real process: a suite whose kills are
//! all simulated cannot catch a runner that keeps something only in memory.
//!
//! Every place the suite narrows a check because the role leaves the rule
//! open is listed in [`NARROWINGS`] and printed with the report.
//!
//! ```ignore
//! let report = run_suite(&my_runner_subject, tempdir.path()).await?;
//! println!("{}", report.render());
//! assert_eq!(report.verdict, SuiteVerdict::Passed);
//! ```

#![forbid(unsafe_code)]

mod cases;
mod crash;
mod drive;
mod report;
mod retention;
mod route;
mod runner;
mod subject;

pub use cortexkit_role_harness as harness;
pub use cortexkit_role_llm_runner as wire;
pub use report::{CaseOutcome, CaseReport, CaseSpec, SuiteReport, SuiteVerdict, CASES, NARROWINGS};
pub use route::{envelope, Reply, RouteFailure, RunnerRoute, SubscribeOutcome};
pub use runner::{run_suite, SetupError};
pub use subject::{
    Capability, LlmRunnerSubject, Script, ScriptedPart, ScriptedToolCall, ScriptedTurn,
};

/// The one tool every scripted call names. The subject's scripted tool
/// provider serves it, and the session plan it builds makes it available.
pub const SCRIPTED_TOOL: &str = "conformance_echo";
