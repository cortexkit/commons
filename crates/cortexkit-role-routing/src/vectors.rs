//! Shared `routing/v1` vectors, compiled in with `test-vectors`.
//!
//! The files live in this crate's `test-vectors/routing-v1/` directory and
//! ship in its package. A provider or consumer can enable this feature to
//! test against the same bytes as this crate's own vector tests.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "routing-v1";

/// Valid and invalid catalog consumer declarations, including canonical bytes.
pub const CONSUMER_DECLARATIONS_JSON: &str =
    include_str!("../test-vectors/routing-v1/consumer-declarations.json");

/// Unknown values of every open set.
pub const OPEN_SETS_JSON: &str = include_str!("../test-vectors/routing-v1/open-sets.json");
/// One refusal body for every refusal code.
pub const REFUSALS_JSON: &str = include_str!("../test-vectors/routing-v1/refusals.json");
/// Reply bodies, wrapped and refused.
pub const REPLY_ENVELOPE_JSON: &str =
    include_str!("../test-vectors/routing-v1/reply-envelope.json");
/// The `role.describe` request and answer.
pub const ROLE_DESCRIBE_JSON: &str = include_str!("../test-vectors/routing-v1/role-describe.json");
/// `route.account_for` requests, refusals, answers and body comparisons.
pub const ROUTE_ACCOUNT_FOR_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-account-for.json");
/// Opt-in continuation requests, all outcomes, retries and mode refusals.
pub const ROUTE_ACCOUNT_CONTINUE_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-account-continue.json");
/// `route.account_observe` requests, refusals and answers.
pub const ROUTE_ACCOUNT_OBSERVE_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-account-observe.json");
/// `route.report` requests, refusals, answers and body comparisons.
pub const ROUTE_REPORT_JSON: &str = include_str!("../test-vectors/routing-v1/route-report.json");
/// `route.select` requests, refusals and answers.
pub const ROUTE_SELECT_JSON: &str = include_str!("../test-vectors/routing-v1/route-select.json");
/// The `select_hold_control` group: a describe declaring it, the select
/// members it adds with and without the group, the `accounts_blocked`
/// refusal, and an answer naming its runner's harness.
pub const ROUTE_SELECT_HOLD_CONTROL_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-hold-control.json");

/// Runner-wide program declarations and their field-path refusals.
pub const RUNNER_DECLARATIONS_JSON: &str =
    include_str!("../test-vectors/routing-v1/runner-declarations.json");
/// The optional `select_harness` group, program answers and no-route refusals.
pub const ROUTE_SELECT_HARNESS_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-harness.json");

/// The optional `select_prefer` group, the preference list with and without it.
pub const ROUTE_SELECT_PREFER_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-prefer.json");

/// The optional `select_exclude_providers` group, the provider exclusion list
/// with and without it.
pub const ROUTE_SELECT_EXCLUDE_PROVIDERS_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-exclude-providers.json");

/// The optional `select_task_ref` group, task references and field-named refusals.
pub const ROUTE_SELECT_TASK_REF_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-task-ref.json");

/// Optional account-allocation members added after the original v1 vectors.
pub const ROUTE_ACCOUNT_FOR_OPTIONAL_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-account-for-optional.json");
/// Optional pool members on observations.
pub const ROUTE_ACCOUNT_OBSERVE_OPTIONAL_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-account-observe-optional.json");
/// Optional report members, conflicts and unknown eligibility scope.
pub const ROUTE_REPORT_OPTIONAL_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-report-optional.json");

/// The `account_status` optional group.
pub const ROUTE_ACCOUNT_STATUS_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-account-status.json");
/// The `budget_quote` optional group.
pub const ROUTE_BUDGET_QUOTE_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-budget-quote.json");
/// The `explain` optional group.
pub const ROUTE_EXPLAIN_JSON: &str = include_str!("../test-vectors/routing-v1/route-explain.json");
/// The `head_choices` optional group.
pub const ROUTE_HEAD_CHOICES_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-head-choices.json");
/// Consumer-specific scores, exclusions, admission refusals and invalid answers.
pub const ROUTE_CONSUMER_MODELS_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-consumer-models.json");
/// Run-attributed usage and field-path validation with and without discovery.
pub const ROUTE_REPORT_USAGE_RUN_ID_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-report-usage-run-id.json");
/// The `observe_backoff` optional group.
pub const ROUTE_OBSERVE_BACKOFF_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-observe-backoff.json");
/// Credential-wide observations with and without their optional group.
pub const ROUTE_OBSERVE_CREDENTIAL_REFUSED_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-observe-credential-refused.json");
/// The `report_outcomes` optional group.
pub const ROUTE_REPORT_OUTCOMES_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-report-outcomes.json");
/// The `select_available_routes` optional group.
pub const ROUTE_SELECT_AVAILABLE_ROUTES_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-available-routes.json");
/// The `select_panel` optional group.
pub const ROUTE_SELECT_PANEL_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-panel.json");
/// The `select_pin_slot` optional group.
pub const ROUTE_SELECT_PIN_SLOT_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-pin-slot.json");
/// The `select_prefer_prompt_cache` optional group.
pub const ROUTE_SELECT_PREFER_PROMPT_CACHE_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-prefer-prompt-cache.json");
/// The `select_settings` optional group.
pub const ROUTE_SELECT_SETTINGS_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-select-settings.json");

/// Every vector file as `(file name, contents)`, in `SHA256SUMS` order.
pub const FILES: &[(&str, &str)] = &[
    ("consumer-declarations.json", CONSUMER_DECLARATIONS_JSON),
    ("open-sets.json", OPEN_SETS_JSON),
    ("refusals.json", REFUSALS_JSON),
    ("reply-envelope.json", REPLY_ENVELOPE_JSON),
    ("role-describe.json", ROLE_DESCRIBE_JSON),
    ("route-account-continue.json", ROUTE_ACCOUNT_CONTINUE_JSON),
    (
        "route-account-for-optional.json",
        ROUTE_ACCOUNT_FOR_OPTIONAL_JSON,
    ),
    ("route-account-for.json", ROUTE_ACCOUNT_FOR_JSON),
    (
        "route-account-observe-optional.json",
        ROUTE_ACCOUNT_OBSERVE_OPTIONAL_JSON,
    ),
    ("route-account-observe.json", ROUTE_ACCOUNT_OBSERVE_JSON),
    ("route-account-status.json", ROUTE_ACCOUNT_STATUS_JSON),
    ("route-budget-quote.json", ROUTE_BUDGET_QUOTE_JSON),
    ("route-consumer-models.json", ROUTE_CONSUMER_MODELS_JSON),
    ("route-explain.json", ROUTE_EXPLAIN_JSON),
    ("route-head-choices.json", ROUTE_HEAD_CHOICES_JSON),
    ("route-observe-backoff.json", ROUTE_OBSERVE_BACKOFF_JSON),
    (
        "route-observe-credential-refused.json",
        ROUTE_OBSERVE_CREDENTIAL_REFUSED_JSON,
    ),
    ("route-report-optional.json", ROUTE_REPORT_OPTIONAL_JSON),
    ("route-report-outcomes.json", ROUTE_REPORT_OUTCOMES_JSON),
    (
        "route-report-usage-run-id.json",
        ROUTE_REPORT_USAGE_RUN_ID_JSON,
    ),
    ("route-report.json", ROUTE_REPORT_JSON),
    (
        "route-select-available-routes.json",
        ROUTE_SELECT_AVAILABLE_ROUTES_JSON,
    ),
    (
        "route-select-exclude-providers.json",
        ROUTE_SELECT_EXCLUDE_PROVIDERS_JSON,
    ),
    ("route-select-harness.json", ROUTE_SELECT_HARNESS_JSON),
    (
        "route-select-hold-control.json",
        ROUTE_SELECT_HOLD_CONTROL_JSON,
    ),
    ("route-select-panel.json", ROUTE_SELECT_PANEL_JSON),
    ("route-select-pin-slot.json", ROUTE_SELECT_PIN_SLOT_JSON),
    (
        "route-select-prefer-prompt-cache.json",
        ROUTE_SELECT_PREFER_PROMPT_CACHE_JSON,
    ),
    ("route-select-prefer.json", ROUTE_SELECT_PREFER_JSON),
    ("route-select-settings.json", ROUTE_SELECT_SETTINGS_JSON),
    ("route-select-task-ref.json", ROUTE_SELECT_TASK_REF_JSON),
    ("route-select.json", ROUTE_SELECT_JSON),
    ("runner-declarations.json", RUNNER_DECLARATIONS_JSON),
];

/// `sha256sum` output for every file in [`FILES`].
pub const SHA256SUMS: &str = include_str!("../test-vectors/routing-v1/SHA256SUMS");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}
