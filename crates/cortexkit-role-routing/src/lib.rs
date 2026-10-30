//! Wire contract of the `routing/v1` role.
//!
//! A routing provider decides which model a piece of model work runs on, and
//! on which runner. A consumer states requirements in `route.select`; the
//! answer is exactly what runs: the consumer sends to the named runner with
//! the named model, and never selects a second time or substitutes. Runners
//! ask the provider which account to use for a call (`route.account_for`)
//! and tell it what they saw on an account (`route.account_observe`).
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document:
//! every rule in it is marked pinned. The types here are its wire shapes:
//!
//! - [`describe`]: the `role.describe` answer, and [`capabilities`] for the
//!   optional groups it declares;
//! - [`select`]: the `route.select` request, its validator, and its answer;
//! - [`report`]: the `route.report` request, its validator, and its answer;
//! - [`account`]: the `route.account_for` request, validator and answer;
//! - [`observe`]: the `route.account_observe` request, validator and answer;
//! - [`declarations`]: consumer defaults and runner matching inputs from the catalog;
//! - [`errors`]: refusal codes, the refusal body and each code's detail;
//! - [`wire`]: the `{"result": <answer>}` reply envelope;
//! - `vectors` (with the `test-vectors` feature): the role's vector files,
//!   compiled in, so a module's tests can use the same bytes.
//!
//! Every enumeration a reader decodes (refusal codes, report outcomes,
//! `pin_unavailable` and `no_route` reasons, observation kinds, account
//! reason codes) is an open set: an unknown value decodes and is kept as the
//! string it was sent as. Optional members are omitted when absent, and a
//! member sent as `null` decodes the same as one left out.

#![forbid(unsafe_code)]

#[macro_use]
mod open;
mod capability_names;
mod members;

pub mod account;
pub mod declarations;
pub mod describe;
pub mod errors;
pub mod observe;
pub mod report;
pub mod select;
pub mod spellings;
#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;
pub mod wire;

pub use declarations::{
    parse_consumer_declarations, ConsumerDeclaration, ConsumerDeclarations, DeclarationError,
    DemandDefaults, RunnerDeclaration, RunnerPair, StandingLimits,
};

/// The role name, as `role.describe` states it.
pub const ROLE: &str = "routing";

/// The only major this crate defines, as a path segment.
pub const VERSION: &str = "v1";

/// The only major this crate defines, as `role.describe`'s `version` number.
pub const VERSION_NUMBER: u32 = 1;

/// The capability identifier a routing provider lists in its manifest's
/// `capabilities.provides`.
pub const PROVIDES: &str = "routing/v1";

/// The daemon catalog's `role_declarations` key for consumer and runner declarations.
pub const ROLE_DECLARATIONS_KEY: &str = "routing/v1";

/// Names of the ops a routing provider serves.
pub mod ops {
    /// The provider's consumers, the requirement fields it reads, and its
    /// ops.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// Choose a model and a runner for one piece of work.
    pub const ROUTE_SELECT: &str = "route.select";
    /// Tell the provider how running on a decision went.
    pub const ROUTE_REPORT: &str = "route.report";
    /// A runner asks which account to use for a model call.
    pub const ROUTE_ACCOUNT_FOR: &str = "route.account_for";
    /// A runner tells the provider what it saw on an account.
    pub const ROUTE_ACCOUNT_OBSERVE: &str = "route.account_observe";
}

/// Ops every `routing/v1` provider serves.
pub const REQUIRED_OPS: &[&str] = &[
    ops::ROLE_DESCRIBE,
    ops::ROUTE_SELECT,
    ops::ROUTE_REPORT,
    ops::ROUTE_ACCOUNT_FOR,
    ops::ROUTE_ACCOUNT_OBSERVE,
];

/// The ops only runner modules call. A consumer that only selects never
/// depends on them.
pub const RUNNER_OPS: &[&str] = &[ops::ROUTE_ACCOUNT_FOR, ops::ROUTE_ACCOUNT_OBSERVE];

/// Optional groups a provider declares in `role.describe`'s `capabilities`.
/// A caller sends a group's members only to a provider that declares the
/// group; a provider that does not declare it refuses them as unknown.
pub mod capabilities {
    /// `route.select` reads the caller's `hold_ok` and `hold_wait_max_s`.
    pub const SELECT_HOLD_CONTROL: &str = "select_hold_control";

    /// `route.select` reads the caller's requested session program, `harness`.
    pub const SELECT_HARNESS: &str = "select_harness";

    /// `route.select` reads the caller's ordered route preference, `prefer`.
    pub const SELECT_PREFER: &str = "select_prefer";

    /// `route.select` reads the caller's `exclude_providers`: provider ids
    /// none of whose routes may be answered.
    pub const SELECT_EXCLUDE_PROVIDERS: &str = "select_exclude_providers";

    /// `route.select` reads `task_ref` for attribution, never selection.
    pub const SELECT_TASK_REF: &str = "select_task_ref";

    /// Every group this version defines.
    pub const KNOWN: &[&str] = &[
        SELECT_HOLD_CONTROL,
        SELECT_HARNESS,
        SELECT_PREFER,
        SELECT_EXCLUDE_PROVIDERS,
        SELECT_TASK_REF,
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provides_is_role_slash_version() {
        assert_eq!(PROVIDES, format!("{ROLE}/{VERSION}"));
        assert_eq!(VERSION, format!("v{VERSION_NUMBER}"));
    }

    #[test]
    fn provides_is_a_valid_capability_identifier() {
        assert!(subc_protocol::manifest::is_valid_capability_identifier(
            PROVIDES
        ));
    }

    #[test]
    fn required_ops_are_the_role_ops() {
        assert_eq!(
            REQUIRED_OPS,
            &[
                "role.describe",
                "route.select",
                "route.report",
                "route.account_for",
                "route.account_observe"
            ]
        );
        for op in RUNNER_OPS {
            assert!(REQUIRED_OPS.contains(op), "{op}");
        }
    }
}
