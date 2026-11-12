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
//! - [`select_engine`]: media selection requests, matching, ranking and answers;
//! - [`engine_declarations`]: atomic media-provider catalog-slot validation;
//! - [`report`]: the `route.report` request, its validator, and its answer;
//! - [`account`]: the `route.account_for` request, validator and answer;
//! - [`observe`]: the `route.account_observe` request, validator and answer;
//! - [`panel`], [`budget`], [`explain`], [`account_status`], [`head_choices`]
//!   and [`consumer_models`]: the optional panel selection and replacement,
//!   budget quote, decision explanation, account status and model-choice operations,
//!   each with its request, validator and answer;
//! - [`declarations`]: consumer defaults and runner matching inputs from the catalog;
//! - [`errors`]: refusal codes, the refusal body and each code's detail;
//! - [`refusals`]: the shared pure retry classification for answered refusals;
//! - [`wire`]: the `{"result": <answer>}` reply envelope;
//! - `vectors` (with the `test-vectors` feature): the role's vector files,
//!   compiled in, so a module's tests can use the same bytes.
//!
//! String enumerations a reader decodes (refusal codes, report outcomes,
//! `pin_unavailable` and `no_route` reasons, observation kinds, account
//! reason codes) is an open set: an unknown value decodes and is kept as the
//! string it was sent as. Optional members are omitted when absent, and a
//! member sent as `null` decodes the same as one left out.
//! Continuation answers are instead a closed tagged union of three outcomes;
//! model-choice [`billing::Billing`] values are also closed.

#![forbid(unsafe_code)]

#[macro_use]
mod open;
mod capability_names;
mod members;

pub mod account;
pub mod account_continue;
pub mod account_status;
pub mod billing;
pub mod budget;
pub mod consumer_models;
pub mod declarations;
pub mod describe;
pub mod engine_declarations;
pub mod engine_measurements;
pub mod errors;
pub mod explain;
pub mod head_choices;
pub mod observe;
pub mod panel;
pub mod refusals;
pub mod report;
pub mod select;
pub mod select_engine;
pub mod service_tier;
pub mod spellings;
#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;
pub mod wire;

pub use declarations::{
    parse_consumer_declarations, ConsumerDeclaration, ConsumerDeclarations, DeclarationError,
    DemandDefaults, HarnessLimits, HarnessLimitsError, RunnerDeclaration, RunnerPair,
    StandingLimits,
};
pub use engine_declarations::{
    parse_engine_declarations, EngineDeclarationError, EngineDeclarations,
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
    /// Choose a media engine for one piece of work.
    pub const ROUTE_SELECT_ENGINE: &str = "route.select_engine";
    /// Report a completed media job's speed measurements.
    pub const ROUTE_ENGINE_REPORT: &str = "route.engine_report";
    /// Report an engine's accuracy on a reference set.
    pub const ROUTE_ENGINE_EVAL: &str = "route.engine_eval";
    /// Choose a multi-model panel or replace one of its seats.
    pub const ROUTE_SELECT_PANEL: &str = "route.select_panel";
    /// Quote package and output limits from retained decision capacities.
    pub const ROUTE_BUDGET_QUOTE: &str = "route.budget_quote";
    /// Explain a retained selection owned by the caller.
    pub const ROUTE_EXPLAIN: &str = "route.explain";
    /// Read an account's public limit state.
    pub const ROUTE_ACCOUNT_STATUS: &str = "route.account_status";
    /// List the router's admitted model choices for a runner.
    pub const ROUTE_HEAD_CHOICES: &str = "route.head_choices";
    /// Read the router's stored judgments for a consumer's models.
    pub const ROUTE_CONSUMER_MODELS: &str = "route.consumer_models";
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

/// The account ops used by attested runner and media-provider modules.
/// A consumer that only selects never depends on them.
pub const RUNNER_OPS: &[&str] = &[ops::ROUTE_ACCOUNT_FOR, ops::ROUTE_ACCOUNT_OBSERVE];

/// Optional groups a provider declares in `role.describe`'s `capabilities`.
/// A caller sends a group's members only to a provider that declares the
/// group; a provider that does not declare it refuses them as unknown.
pub mod capabilities {
    /// Media engine selection through `route.select_engine`.
    pub const SELECT_ENGINE: &str = "select_engine";
    /// Per-job engine speed reports. Requires `SELECT_ENGINE`.
    pub const ENGINE_REPORTS: &str = "engine_reports";
    /// Reference-set engine accuracy evaluations. Requires `SELECT_ENGINE`.
    pub const ENGINE_EVALS: &str = "engine_evals";
    /// Switch a durably paused run to a compatible account via `route.account_for`.
    pub const ACCOUNT_CONTINUE: &str = "account_continue";
    /// Preserve a fresh conversation's last-used credential on admission.
    pub const ACCOUNT_WARM_CREDENTIAL: &str = "account_warm_credential";
    /// Tier-aware selection and tier metadata in report usage. A caller must
    /// check this group before sending any of those members to a router.
    pub const SERVICE_TIER: &str = "service_tier";
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

    /// Multi-model panels and seat replacements.
    pub const SELECT_PANEL: &str = "select_panel";
    /// Package and output-limit quotes for retained selections.
    pub const BUDGET_QUOTE: &str = "budget_quote";
    /// Explanations of retained selections.
    pub const EXPLAIN: &str = "explain";
    /// Public account limit state.
    pub const ACCOUNT_STATUS: &str = "account_status";
    /// Admitted model choices for a runner.
    pub const HEAD_CHOICES: &str = "head_choices";
    /// Stored scores and exclusions for a consumer's models.
    pub const CONSUMER_MODELS: &str = "consumer_models";
    /// Operator-policy slot pins in selection.
    pub const SELECT_PIN_SLOT: &str = "select_pin_slot";
    /// Selection restricted to caller-reachable routes.
    pub const SELECT_AVAILABLE_ROUTES: &str = "select_available_routes";
    /// Prompt-cache preference among adequate routes.
    pub const SELECT_PREFER_PROMPT_CACHE: &str = "select_prefer_prompt_cache";
    /// Project and service-tier settings lookup in selection.
    pub const SELECT_SETTINGS: &str = "select_settings";
    /// Account-local backoff observations and decision-owner authorization.
    pub const OBSERVE_BACKOFF: &str = "observe_backoff";
    /// Credential-wide refusal observations. Requires `OBSERVE_BACKOFF`.
    pub const OBSERVE_CREDENTIAL_REFUSED: &str = "observe_credential_refused";
    /// Report outcomes that distinguish failures before a provider send.
    pub const REPORT_OUTCOMES: &str = "report_outcomes";
    /// Attribute report usage to individual runs of a decision.
    pub const REPORT_USAGE_RUN_ID: &str = "report_usage_run_id";

    /// Optional operations and their groups. Each pair must be declared together.
    pub const OP_GROUPS: &[(&str, &str)] = &[
        (super::ops::ROUTE_SELECT_ENGINE, SELECT_ENGINE),
        (super::ops::ROUTE_ENGINE_REPORT, ENGINE_REPORTS),
        (super::ops::ROUTE_ENGINE_EVAL, ENGINE_EVALS),
        (super::ops::ROUTE_SELECT_PANEL, SELECT_PANEL),
        (super::ops::ROUTE_BUDGET_QUOTE, BUDGET_QUOTE),
        (super::ops::ROUTE_EXPLAIN, EXPLAIN),
        (super::ops::ROUTE_ACCOUNT_STATUS, ACCOUNT_STATUS),
        (super::ops::ROUTE_HEAD_CHOICES, HEAD_CHOICES),
        (super::ops::ROUTE_CONSUMER_MODELS, CONSUMER_MODELS),
    ];

    /// Every group this version defines.
    pub const KNOWN: &[&str] = &[
        SELECT_ENGINE,
        ENGINE_REPORTS,
        ENGINE_EVALS,
        ACCOUNT_CONTINUE,
        ACCOUNT_WARM_CREDENTIAL,
        SERVICE_TIER,
        SELECT_HOLD_CONTROL,
        SELECT_HARNESS,
        SELECT_PREFER,
        SELECT_EXCLUDE_PROVIDERS,
        SELECT_TASK_REF,
        SELECT_PANEL,
        BUDGET_QUOTE,
        EXPLAIN,
        ACCOUNT_STATUS,
        HEAD_CHOICES,
        CONSUMER_MODELS,
        SELECT_PIN_SLOT,
        SELECT_AVAILABLE_ROUTES,
        SELECT_PREFER_PROMPT_CACHE,
        SELECT_SETTINGS,
        OBSERVE_BACKOFF,
        OBSERVE_CREDENTIAL_REFUSED,
        REPORT_OUTCOMES,
        REPORT_USAGE_RUN_ID,
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
