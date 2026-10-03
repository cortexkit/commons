//! Conformance suite for the `classifier/v1` role.
//!
//! A classifier module runs this suite in its own CI, against its real
//! module over its real management route. The module's provider is a
//! stand-in the module's CI wires in, so the suite never calls a real
//! provider: it scripts what the stand-in answers for each item and reads
//! how many times the stand-in was called for it. See `README.md` for what
//! a module supplies and [`CHECKS`] for what each check asserts.
//!
//! The suite is never run against a double; the in-process fake under
//! `tests/fake/` exists only to test the suite itself.

#![forbid(unsafe_code)]

mod checks;
mod report;
mod route;
mod runner;
mod subject;

pub use cortexkit_role_classifier as wire;
pub use report::{
    CheckOutcome, CheckReport, CheckSpec, SuiteReport, SuiteVerdict, CHECKS, NARROWINGS,
};
pub use route::{ClassifierRoute, Reply, RouteFailure};
pub use runner::run_suite;
pub use subject::{state_key, ClassifierSubject, Scripted, ServedModel, MAX_POLLS};
