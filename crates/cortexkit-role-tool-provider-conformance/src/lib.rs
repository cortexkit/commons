//! Conformance runner for the `tool-provider/v1` role.
//!
//! A provider runs this suite in its own CI against its real module over a
//! real route, never against a double. It supplies:
//!
//! - a [`cortexkit_role_harness::Harness`] that spawns the module on a state
//!   root, opens routes to it under a given stamp, kills it at the role's
//!   named points and restarts it on the same root;
//! - a [`ToolRoute`] adapter for its route type, which sends one request over
//!   the real route and reports every frame the module answered with;
//! - the [`ToolProviderSubject`] facts the runner cannot know: which tools to
//!   call, which capabilities the provider and harness have, and a stand-in
//!   answer for an approval question.
//!
//! [`run_suite`] then runs every case in [`CASES`]. A case whose `requires`
//! the subject does not declare is reported as skipped with the missing
//! capability, never as passed. The run fails if any case fails, or if kills
//! were made and none of them ended a real process: a suite whose kills are
//! all simulated cannot catch a provider that keeps something only in memory.
//!
//! ```ignore
//! let report = run_suite(&my_provider_subject, tempdir.path()).await?;
//! println!("{}", report.render());
//! assert_eq!(report.verdict, SuiteVerdict::Passed);
//! ```

#![forbid(unsafe_code)]

mod report;
mod route;
mod runner;
mod subject;

pub use cortexkit_role_harness as harness;
pub use cortexkit_role_tool_provider as wire;
pub use report::{CaseOutcome, CaseReport, CaseSpec, SuiteReport, SuiteVerdict, CASES};
pub use route::{
    single_terminal, Exchange, ObservedFrame, RouteFailure, TerminalProblem, ToolRoute,
};
pub use runner::{run_suite, SetupError};
pub use subject::{CallSpec, Capability, ScopedPrincipals, ToolProviderSubject};
