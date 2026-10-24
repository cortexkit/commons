//! Conformance kit for the `routing/v1` role.
//!
//! A routing provider runs this kit in its own CI, against its real module
//! over real routes. The provider's test harness implements
//! [`RoutingSubject`]: it opens routes under the caller identities and
//! scope stamps the kit names, and sets up the provider's consumers,
//! catalog, project pins and cooldowns through hooks. See `README.md` for
//! what a provider supplies and [`CHECKS`] for what each check asserts.
//!
//! The kit is never run against a double; the in-process fake under
//! `tests/fake/` exists only to test the kit itself.

#![forbid(unsafe_code)]

mod checks;
pub mod fixture;
mod report;
mod route;
mod runner;
mod subject;

pub use cortexkit_role_routing as wire;
pub use report::{
    CheckOutcome, CheckReport, CheckSpec, SuiteReport, SuiteVerdict, CHECKS, NARROWINGS,
};
pub use route::{Reply, RouteFailure, RoutingRoute};
pub use runner::run_suite;
pub use subject::{
    Catalog, CatalogAccount, CatalogModel, CatalogModule, CatalogRunner, ConsumerConfig,
    OperatorRequirements, ReachableToolProvider, RouteStamp, RoutingSubject, ScopeStamp,
};
