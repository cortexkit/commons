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
/// `route.account_observe` requests, refusals and answers.
pub const ROUTE_ACCOUNT_OBSERVE_JSON: &str =
    include_str!("../test-vectors/routing-v1/route-account-observe.json");
/// `route.report` requests, refusals, answers and body comparisons.
pub const ROUTE_REPORT_JSON: &str = include_str!("../test-vectors/routing-v1/route-report.json");
/// `route.select` requests, refusals and answers.
pub const ROUTE_SELECT_JSON: &str = include_str!("../test-vectors/routing-v1/route-select.json");

/// Every vector file as `(file name, contents)`, in `SHA256SUMS` order.
pub const FILES: &[(&str, &str)] = &[
    ("consumer-declarations.json", CONSUMER_DECLARATIONS_JSON),
    ("open-sets.json", OPEN_SETS_JSON),
    ("refusals.json", REFUSALS_JSON),
    ("reply-envelope.json", REPLY_ENVELOPE_JSON),
    ("role-describe.json", ROLE_DESCRIBE_JSON),
    ("route-account-for.json", ROUTE_ACCOUNT_FOR_JSON),
    ("route-account-observe.json", ROUTE_ACCOUNT_OBSERVE_JSON),
    ("route-report.json", ROUTE_REPORT_JSON),
    ("route-select.json", ROUTE_SELECT_JSON),
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
