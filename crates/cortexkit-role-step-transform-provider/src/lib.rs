//! Wire contract of the `step-transform-provider/v1` role.
//!
//! A step-transform provider makes write-time changes to the newest message
//! of a session, through four hooks the runner calls ([`subscription::Hook`]):
//! `pre_user` on every user message and every steered or queued prompt,
//! `post_assistant` when an assistant message completes, `pre_tool` before a
//! tool call executes (in three phases, `mutate`, `validate` and `approve`),
//! and `post_tool` on a tool's result. The runner writes each hook's output
//! on the record it transforms, before anything carrying it is sent, and
//! never calls the hook again for a record that is durable.
//!
//! A provider claims the role by listing [`PROVIDES`] in its manifest's
//! `capabilities.provides` and answers every op in [`REQUIRED_OPS`]. Its
//! declaration ([`ops::TRANSFORM_DECLARE`]) bounds what a session's plan may
//! subscribe it to: for each hook (and `pre_tool` phase), the tools it can
//! act on, the operations it may return, what the runner does when it is
//! unavailable (`on_unavailable`) and its time budget (`budget_ms`). A plan
//! may subscribe it as declared or stricter: a subset of the tools or
//! operations, a smaller budget, or `refuse` where the declaration says
//! `pass`.
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document.
//! The types here are the wire shapes it describes. Requests a provider decodes are lenient on fields and strict on
//! the hook and phase values. Answers the runner decodes are strict on the
//! `answer` and `op` values, because a runner cannot apply an operation it
//! does not understand.

#![forbid(unsafe_code)]

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

pub mod answer;
pub mod describe;
pub mod errors;
pub mod grant;
pub mod hook;
pub mod points;
pub mod subscription;

/// The role name as `role.describe` reports it.
pub const ROLE: &str = "step-transform-provider";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The string a provider lists in its manifest's `capabilities.provides`.
pub const PROVIDES: &str = "step-transform-provider/v1";

/// Names of the ops, as the `method` of the request that carries them.
pub mod ops {
    /// Discovery before use: the role's majors, ops and capabilities.
    /// Required.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// The subscriptions a plan item may choose from, for the item's preset
    /// and params: per hook, the tools, operations, `on_unavailable` and
    /// `budget_ms` a plan may use, or something stricter. Fetched with the
    /// plan's other items during admission, the plan check before accepting
    /// a session (CONTRACT.md §4).
    /// Required.
    pub const TRANSFORM_DECLARE: &str = "transform.declare";
    /// One hook call. Required.
    pub const TRANSFORM_HOOK: &str = "transform.hook";
}

/// Ops every `step-transform-provider/v1` module must list in
/// `role.describe`. A consumer refuses a module missing any of them, by
/// name, before routing anything to it.
pub const REQUIRED_OPS: &[&str] = &[
    ops::ROLE_DESCRIBE,
    ops::TRANSFORM_DECLARE,
    ops::TRANSFORM_HOOK,
];

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
        "declare.json",
        "errors.json",
        "grants.json",
        "hook-answers.json",
        "hook-requests.json",
        "host-runner.json",
        "role-describe.json",
        "subscriptions.json",
    ];

    pub fn dir() -> String {
        format!(
            "{}/../../test-vectors/step-transform-provider-v1",
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
}
