//! A transport-free conformance kit for a live `agent-inbox/v1` implementer.
//!
//! Implement [`AgentInboxSubject`] using real routes and an independent
//! receiver. [`run`] returns stable named outcomes and explicit narrowings.
//! The synchronous adapter may bridge an implementer's own async runtime;
//! this crate brings no runtime or transport dependency into its build.

#![forbid(unsafe_code)]

mod cases;
mod report;
mod runner;
mod subject;

pub use report::{CaseOutcome, CaseReport, CaseSpec, SuiteReport, SuiteVerdict, CASES, NARROWINGS};
pub use runner::run;
pub use subject::{AgentInboxSubject, Caller, Delivery, ScopeStatus};
