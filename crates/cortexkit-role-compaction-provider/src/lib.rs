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
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document.
//! The types here are the wire shapes it describes. Requests a provider decodes are lenient on fields, so a newer
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
        "host-runner-lane.json",
        "host-runner.json",
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
    use serde_json::Value;

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

    /// Every request in the `requests` lists of `setup.json` (Setup) and
    /// `status.json` (step) names its caller's harness, and the field
    /// survives a round trip.
    #[test]
    fn every_request_kind_round_trips_its_harness() {
        let setup = vectors::load("setup.json");
        for case in vectors::cases(&setup, "requests") {
            let name = case["name"].as_str().unwrap();
            let request = vectors::round_trip::<setup::SetupRequest>(name, &case["request"]);
            assert_eq!(request.harness, "broca", "{name}");
        }
        let status = vectors::load("status.json");
        for case in vectors::cases(&status, "requests") {
            let name = case["name"].as_str().unwrap();
            let request = vectors::round_trip::<status::StepStatus>(name, &case["request"]);
            assert_eq!(request.harness, "broca", "{name}");
        }
    }

    /// `harness` is required: removing it from any request in the
    /// `requests` lists of `setup.json` and `status.json` makes the request
    /// fail to decode, and the error names the field.
    #[test]
    fn a_request_without_harness_is_refused_by_name() {
        fn refused<T: serde::de::DeserializeOwned + std::fmt::Debug>(name: &str, request: &Value) {
            let mut request = request.clone();
            assert!(
                request.as_object_mut().unwrap().remove("harness").is_some(),
                "{name}"
            );
            let error = serde_json::from_value::<T>(request)
                .expect_err(name)
                .to_string();
            assert!(error.contains("missing field `harness`"), "{name}: {error}");
        }
        let setup = vectors::load("setup.json");
        for case in vectors::cases(&setup, "requests") {
            refused::<setup::SetupRequest>(case["name"].as_str().unwrap(), &case["request"]);
        }
        let status = vectors::load("status.json");
        for case in vectors::cases(&status, "requests") {
            refused::<status::StepStatus>(case["name"].as_str().unwrap(), &case["request"]);
        }
    }

    #[test]
    fn runner_group_is_the_runner_roles_spelling() {
        assert_eq!(RUNNER_GROUP, "compaction");
        assert!(
            cortexkit_role_llm_runner::capabilities::group_ops(RUNNER_GROUP)
                .is_some_and(|ops| ops.contains(&ops::COMPACTION_READY))
        );
    }

    fn joint_dir() -> String {
        format!("{}/joint", vectors::dir())
    }

    fn sha256(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    #[test]
    fn joint_fixture_files_match_sha256sums() {
        const FILES: &[&str] = &["transcript.json", "exchanges.json", "README.md"];
        let sums_path = format!("{}/SHA256SUMS", joint_dir());
        let sums = std::fs::read_to_string(&sums_path)
            .unwrap_or_else(|error| panic!("{sums_path}: {error}"));
        let checksums: std::collections::HashMap<&str, &str> = sums
            .lines()
            .map(|line| {
                let (hash, name) = line
                    .split_once("  ")
                    .unwrap_or_else(|| panic!("invalid checksum entry: {line}"));
                (name, hash)
            })
            .collect();
        assert_eq!(
            checksums.len(),
            FILES.len(),
            "unexpected SHA256SUMS entries"
        );

        for name in FILES {
            let path = format!("{}/{name}", joint_dir());
            let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
            let expected = checksums
                .get(name)
                .unwrap_or_else(|| panic!("{name}: missing from SHA256SUMS"));
            assert_eq!(sha256(&bytes), *expected, "{name}: SHA-256 mismatch");
        }
    }

    #[derive(serde::Deserialize)]
    struct JointExchange {
        name: String,
        call: String,
        request: Value,
        answer: Value,
        expected_view: Value,
    }

    fn decode_joint<T: serde::de::DeserializeOwned>(name: &str, part: &str, json: &Value) -> T {
        serde_json::from_value(json.clone())
            .unwrap_or_else(|error| panic!("{name}: {part} does not decode: {error}"))
    }

    fn joint_exchanges() -> Vec<JointExchange> {
        let path = format!("{}/exchanges.json", joint_dir());
        let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path}: {error}"));
        serde_json::from_slice(&bytes).unwrap_or_else(|error| panic!("{path}: {error}"))
    }

    #[test]
    fn joint_fixture_exchanges_decode_with_the_wire_types_for_each_call() {
        use cortexkit_role_step_transform_provider::{answer::HookAnswer, hook::HookCall};

        let exchanges = joint_exchanges();
        let names: Vec<&str> = exchanges
            .iter()
            .map(|exchange| exchange.name.as_str())
            .collect();
        assert_eq!(
            names,
            [
                "setup-initial",
                "tag-strip-mixed",
                "reminder-last-text-block",
                "noop-keeps-view",
                "known-refusal",
                "unknown-refusal-control",
                "oversized-text-keeps-view",
            ]
        );

        for exchange in &exchanges {
            let method = exchange.request["method"].as_str().unwrap_or_else(|| {
                panic!(
                    "{}: request method is missing or not a string",
                    exchange.name
                )
            });
            match method {
                "compaction.setup" => {
                    assert_eq!(exchange.call, "setup", "{}", exchange.name);
                    decode_joint::<OpRequest<setup::SetupRequest>>(
                        &exchange.name,
                        "request",
                        &exchange.request,
                    );
                    decode_joint::<setup::SetupAnswer>(&exchange.name, "answer", &exchange.answer);
                }
                "compaction.step" => {
                    assert_eq!(exchange.call, "step", "{}", exchange.name);
                    decode_joint::<OpRequest<status::StepStatus>>(
                        &exchange.name,
                        "request",
                        &exchange.request,
                    );
                    decode_joint::<answer::StepAnswer>(&exchange.name, "answer", &exchange.answer);
                }
                "transform.hook" => {
                    assert_eq!(exchange.call, "step", "{}", exchange.name);
                    decode_joint::<cortexkit_role_step_transform_provider::OpRequest<HookCall>>(
                        &exchange.name,
                        "request",
                        &exchange.request,
                    );
                    decode_joint::<HookAnswer>(&exchange.name, "answer", &exchange.answer);
                }
                other => panic!("{}: unexpected call method {other}", exchange.name),
            }
            let _ = &exchange.expected_view;
        }
    }

    #[test]
    fn joint_unknown_refusal_control_decodes_as_an_unknown_refusal_code() {
        let exchange = joint_exchanges()
            .into_iter()
            .find(|exchange| exchange.name == "unknown-refusal-control")
            .expect("unknown-refusal-control exchange is present");
        let answer = decode_joint::<setup::SetupAnswer>(&exchange.name, "answer", &exchange.answer);
        match answer {
            setup::SetupAnswer::Refuse { code, .. } => {
                assert_eq!(
                    code,
                    errors::RefuseCode::Unknown("future_provider_reason".into())
                );
            }
            other => panic!("{}: expected refusal, got {other:?}", exchange.name),
        }
    }
}
