//! Wire contract of the `compaction-provider/v1` role.
//!
//! A compaction provider decides the shape of the history a runner sends to
//! the model. A session has at most one. The runner calls it: once when the
//! session starts ([`ops::COMPACTION_SETUP`]), and then for the steps its
//! declared conditions select ([`ops::COMPACTION_STEP`]). Each step call
//! carries a status and gets one of four answers: `noop`, a
//! CompactionMessage that replaces one contiguous range of messages, `wait`,
//! or `refuse`. After a `wait`, the provider tells the runner it is done with
//! `compaction.ready`, an op the runner serves.
//!
//! A provider claims the role by listing [`PROVIDES`] in its manifest's
//! `capabilities.provides` and answers every op in [`REQUIRED_OPS`].
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document. The types here are the wire shapes it
//! describes. Requests a provider decodes are lenient on fields, so a newer
//! runner degrades against an older provider. Answers the runner decodes are
//! strict on the `answer` value, because a runner cannot act on an answer it
//! does not understand.

#![forbid(unsafe_code)]

pub mod answer;
pub mod describe;
pub mod errors;
pub mod fence;
pub mod points;
pub mod ready;
pub mod setup;
pub mod status;

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

/// Names of the ops, as the `method` of the request that carries them.
pub mod ops {
    /// Discovery before use: the role's majors, ops and capabilities.
    /// Required.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// Called once per session before its first model call. Returns the
    /// initial CompactionMessage, the stability ranks of the provider's own
    /// messages and the conditions for calling it. Required.
    pub const COMPACTION_SETUP: &str = "compaction.setup";
    /// The per-step call: a status in, one of four answers out. Required.
    pub const COMPACTION_STEP: &str = "compaction.step";
    /// Served by the RUNNER, not the provider: the provider's signal that
    /// the work behind a `wait` answer is done. Listed here because the
    /// provider sends it.
    pub const COMPACTION_READY: &str = "compaction.ready";
}

/// Ops every `compaction-provider/v1` module must list in `role.describe`.
/// A consumer refuses a module missing any of them, by name, before routing
/// anything to it.
pub const REQUIRED_OPS: &[&str] = &[
    ops::ROLE_DESCRIBE,
    ops::COMPACTION_SETUP,
    ops::COMPACTION_STEP,
];

/// The `llm-runner/v1` capability group a runner declares when it can host
/// a compaction provider. A runner without it cannot be paired with a
/// compaction item, and refuses such a plan at admission (the plan check
/// before accepting a session, CONTRACT.md §3). Taken from the
/// runner role's crate so the two spellings cannot drift apart.
pub const RUNNER_GROUP: &str = cortexkit_role_llm_runner::capabilities::COMPACTION;

#[cfg(test)]
pub(crate) mod vectors {
    //! The shared test vectors, read from the repository's `test-vectors`
    //! directory so the wire crate, a later conformance runner and any other
    //! implementation check the same bytes.

    use serde::{de::DeserializeOwned, Serialize};
    use serde_json::Value;

    /// Every vector file. A test checks this list against the directory, so
    /// a new file cannot land without a test reading it.
    pub const FILES: &[&str] = &[
        "answers.json",
        "errors.json",
        "fence.json",
        "ready.json",
        "role-describe.json",
        "setup.json",
        "status.json",
    ];

    pub fn dir() -> String {
        format!(
            "{}/../../test-vectors/compaction-provider-v1",
            env!("CARGO_MANIFEST_DIR")
        )
    }

    pub fn load(name: &str) -> Value {
        assert!(FILES.contains(&name), "{name} is not listed in FILES");
        let path = format!("{}/{name}", dir());
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
    }

    /// The array at `key`, panicking with the key's name if it is missing or
    /// not an array.
    pub fn cases<'a>(file: &'a Value, key: &str) -> &'a Vec<Value> {
        file[key]
            .as_array()
            .unwrap_or_else(|| panic!("vector key {key} is not an array"))
    }

    /// Decode `json` as `T`, then check that encoding it gives back exactly
    /// `json` and that decoding the encoding gives the same value. Canonical
    /// vectors carry no unknown fields, so a field the type drops or renames
    /// shows up here.
    pub fn round_trip<T>(name: &str, json: &Value) -> T
    where
        T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug,
    {
        let decoded: T = serde_json::from_value(json.clone())
            .unwrap_or_else(|e| panic!("{name}: does not decode: {e}"));
        let encoded = serde_json::to_value(&decoded).unwrap();
        assert_eq!(&encoded, json, "{name}: encoding differs from the vector");
        let again: T = serde_json::from_value(encoded).unwrap();
        assert_eq!(again, decoded, "{name}: second decode differs");
        decoded
    }

    /// Check that `json` does not decode as `T`.
    pub fn refused<T: DeserializeOwned + std::fmt::Debug>(name: &str, json: &Value) {
        if let Ok(decoded) = serde_json::from_value::<T>(json.clone()) {
            panic!("{name}: decoded, expected a refusal: {decoded:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_vector_file_on_disk_is_listed() {
        let mut on_disk: Vec<String> = std::fs::read_dir(vectors::dir())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name.ends_with(".json"))
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = vectors::FILES.iter().map(|s| s.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
    }

    #[test]
    fn runner_group_is_the_runner_roles_spelling() {
        assert_eq!(RUNNER_GROUP, "compaction");
        assert!(
            cortexkit_role_llm_runner::capabilities::group_ops(RUNNER_GROUP)
                .is_some_and(|ops| ops.contains(&ops::COMPACTION_READY))
        );
    }
}
