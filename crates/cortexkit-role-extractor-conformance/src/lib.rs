//! Conformance suite for the request side of the `extractor/v1` role.
//!
//! An extractor module runs this suite in its own CI, against its real
//! module over its real management route. The module's provider is a
//! stand-in the module's CI wires in, so the suite never calls a real
//! provider: it reads how many times the stand-in was called for an item,
//! and what it received. The suite checks request validation and
//! admission: what the module refuses, with which code and field, that a
//! refusal reaches no provider and writes nothing, and what the module
//! sends the provider for a request it admits. See `README.md` for what a
//! module supplies and [`CHECKS`] for what each check asserts.
//!
//! The suite is never run against a double; the in-process fake under
//! `tests/fake/` exists only to test the suite itself.

#![forbid(unsafe_code)]

mod checks;
mod report;
mod route;
mod runner;
mod subject;

pub use cortexkit_role_extractor as wire;
pub use report::{
    CheckOutcome, CheckReport, CheckSpec, SuiteReport, SuiteVerdict, CHECKS, NARROWINGS,
};
pub use route::{ExtractorRoute, Reply, RouteFailure};
pub use runner::run_suite;
pub use subject::{ExtractorSubject, ServedModel};
