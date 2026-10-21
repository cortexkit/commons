//! Wire contract of the `compaction-provider/v1` role.
//!
//! A compaction provider decides the shape of the history a runner sends to
//! the model, off the runner's hot path. A session has at most one. The
//! runner calls Setup once when the session starts ([`ops::COMPACTION_SETUP`]),
//! and gets the initial CompactionMessage and the ceiling policy. After that:
//!
//! - after each write, the runner publishes an advance notice on ck-bus
//!   ([`notice::AdvanceNotice`]). The bus carries ids, digests and headers,
//!   never message content, so the notice only wakes the provider, which
//!   then reads the new messages over its `llm-runner/v1` route with
//!   `session.read`;
//! - whenever it decides to, the provider publishes that a view is ready
//!   ([`notice::ViewReady`]); the runner fetches it with [`ops::VIEW_GET`],
//!   checks its digest, persists it and applies it at every step boundary
//!   until a newer one arrives ([`view::CompactionView`]);
//! - when the runner's prefix is being rewritten anyway, or a request reached
//!   the ceiling, it asks for a view synchronously with
//!   [`ops::COMPACTION_RIDE`] ([`ride::RideRequest`]);
//! - Setup's ceiling ([`ceiling::CeilingPolicy`]) says when the runner pauses
//!   for a view and, failing one, refuses the run with `context_ceiling`.
//!
//! A provider claims the role by listing [`PROVIDES`] in its manifest's
//! `capabilities.provides` and answers every op in [`REQUIRED_OPS`].
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document.
//! The types here are the wire shapes it describes. Requests a provider
//! decodes are lenient on fields, so a newer runner degrades against an
//! older provider. Answers the runner decodes are strict on the `answer`
//! value, because a runner cannot act on an answer it does not understand.

#![forbid(unsafe_code)]

pub mod ceiling;
pub mod describe;
pub mod digest;
pub mod errors;
pub mod lineage;
pub mod message;
pub mod notice;
pub mod points;
pub mod ride;
pub mod setup;
pub mod view;

#[cfg(test)]
mod vector_tests;

/// The op envelope of every request in this role, `{method, params}`:
/// `method` is the op's name ([`ops`]) and `params` its request.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[non_exhaustive]
pub struct OpRequest<T> {
    pub method: String,
    pub params: T,
}

impl<T> OpRequest<T> {
    pub fn new(method: impl Into<String>, params: T) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

/// The role name as `role.describe` reports it.
pub const ROLE: &str = "compaction-provider";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The string a provider lists in its manifest's `capabilities.provides`.
pub const PROVIDES: &str = "compaction-provider/v1";

/// Names of the ops a provider serves, as the `method` of the request that
/// carries them. All of them are required.
pub mod ops {
    /// Discovery before use: the role's majors, ops and capabilities.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// Called once per session before its first model call. Returns the
    /// initial CompactionMessage and the ceiling policy.
    pub const COMPACTION_SETUP: &str = "compaction.setup";
    /// Fetch one published view by version.
    pub const VIEW_GET: &str = "view.get";
    /// The runner's synchronous request for a view when its prefix is being
    /// rewritten anyway or a request reached the ceiling.
    pub const COMPACTION_RIDE: &str = "compaction.ride";
}

/// Ops every `compaction-provider/v1` module must list in `role.describe`.
/// A consumer refuses a module missing any of them, by name, before routing
/// anything to it.
pub const REQUIRED_OPS: &[&str] = &[
    ops::ROLE_DESCRIBE,
    ops::COMPACTION_SETUP,
    ops::VIEW_GET,
    ops::COMPACTION_RIDE,
];

/// The `llm-runner/v1` capability group a runner declares when it can host
/// a compaction provider, that is, when it implements the runner's side of
/// this role: advance notices, applying views, rides and the ceiling
/// (CONTRACT.md §3). A runner without it has no compaction provider, and
/// refuses a plan naming one at admission. Taken from the runner role's
/// crate so the two spellings cannot drift apart.
pub const RUNNER_GROUP: &str = cortexkit_role_llm_runner::capabilities::COMPACTION;

#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn every_vector_file_on_disk_is_listed() {
        let mut on_disk: Vec<String> = std::fs::read_dir(vectors::dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".json"))
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = vectors::FILES
            .iter()
            .filter(|(name, _)| name.ends_with(".json"))
            .map(|(name, _)| name.to_string())
            .collect();
        listed.sort();
        assert_eq!(on_disk, listed);
    }

    /// Every request in the `requests` list of `setup.json` names its
    /// caller's harness, and the field survives a round trip.
    #[test]
    fn every_setup_request_round_trips_its_harness() {
        let setup = vectors::load("setup.json");
        for case in vectors::cases(&setup, "requests") {
            let name = case["name"].as_str().unwrap();
            let request = vectors::round_trip::<setup::SetupRequest>(name, &case["request"]);
            assert_eq!(request.harness, "broca", "{name}");
        }
    }

    /// `harness` is required: removing it from any Setup request in
    /// `setup.json` makes the request fail to decode, and the error names
    /// the field.
    #[test]
    fn a_setup_request_without_harness_is_refused_by_name() {
        let setup = vectors::load("setup.json");
        for case in vectors::cases(&setup, "requests") {
            let name = case["name"].as_str().unwrap();
            let mut request: Value = case["request"].clone();
            assert!(
                request.as_object_mut().unwrap().remove("harness").is_some(),
                "{name}"
            );
            let error = serde_json::from_value::<setup::SetupRequest>(request)
                .expect_err(name)
                .to_string();
            assert!(error.contains("missing field `harness`"), "{name}: {error}");
        }
    }

    #[test]
    fn runner_group_is_the_runner_roles_spelling() {
        assert_eq!(RUNNER_GROUP, "compaction");
        assert!(cortexkit_role_llm_runner::capabilities::group_ops(RUNNER_GROUP).is_some());
    }

    #[test]
    fn setup_descent_fixture_matches_sha256sum() {
        let sums = std::fs::read_to_string(format!("{}/SETUP_DESCENT_SHA256SUMS", vectors::dir()))
            .unwrap();
        let lines: Vec<_> = sums.lines().collect();
        assert_eq!(lines.len(), 1);
        let (digest, name) = lines[0].split_once("  ").unwrap();
        assert_eq!(name, "setup-descent.json");
        assert_eq!(
            digest::sha256_hex(&std::fs::read(format!("{}/{name}", vectors::dir())).unwrap()),
            digest,
            "{name}"
        );
    }
}
