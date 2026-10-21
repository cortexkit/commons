//! Shared `step-transform-provider-v1` vectors, compiled in with `test-vectors`.
//! These are the same bytes the wire crate tests and its package ships.

#[cfg(test)]
use serde::{de::DeserializeOwned, Serialize};
#[cfg(test)]
use serde_json::Value;

pub const DIR: &str = "step-transform-provider-v1";
pub const README: &str = include_str!("../test-vectors/step-transform-provider-v1/README.md");
pub const SHA256SUMS: &str = include_str!("../test-vectors/step-transform-provider-v1/SHA256SUMS");
/// Every shipped vector file, in SHA256SUMS order.
pub const FILES: &[(&str, &str)] = &[
    (
        "README.md",
        include_str!("../test-vectors/step-transform-provider-v1/README.md"),
    ),
    (
        "CLAUDE_CODE_SHA256SUMS",
        include_str!("../test-vectors/step-transform-provider-v1/CLAUDE_CODE_SHA256SUMS"),
    ),
    (
        "claude-code-native-ingress.json",
        include_str!("../test-vectors/step-transform-provider-v1/claude-code-native-ingress.json"),
    ),
    (
        "claude-code.json",
        include_str!("../test-vectors/step-transform-provider-v1/claude-code.json"),
    ),
    (
        "declare.json",
        include_str!("../test-vectors/step-transform-provider-v1/declare.json"),
    ),
    (
        "errors.json",
        include_str!("../test-vectors/step-transform-provider-v1/errors.json"),
    ),
    (
        "grants.json",
        include_str!("../test-vectors/step-transform-provider-v1/grants.json"),
    ),
    (
        "hook-answers.json",
        include_str!("../test-vectors/step-transform-provider-v1/hook-answers.json"),
    ),
    (
        "hook-requests.json",
        include_str!("../test-vectors/step-transform-provider-v1/hook-requests.json"),
    ),
    (
        "host-runner-lane.json",
        include_str!("../test-vectors/step-transform-provider-v1/host-runner-lane.json"),
    ),
    (
        "host-runner.json",
        include_str!("../test-vectors/step-transform-provider-v1/host-runner.json"),
    ),
    (
        "role-describe.json",
        include_str!("../test-vectors/step-transform-provider-v1/role-describe.json"),
    ),
    (
        "subscriptions.json",
        include_str!("../test-vectors/step-transform-provider-v1/subscriptions.json"),
    ),
];

pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}

#[cfg(test)]
pub fn dir() -> String {
    format!("{}/test-vectors/{}", env!("CARGO_MANIFEST_DIR"), DIR)
}

#[cfg(test)]
pub fn load(name: &str) -> Value {
    serde_json::from_str(file(name).unwrap_or_else(|| panic!("unknown vector {name}"))).unwrap()
}

/// The array at `key`, panicking with the key's name if it is missing or
/// not an array.
#[cfg(test)]
pub fn cases<'a>(file: &'a Value, key: &str) -> &'a Vec<Value> {
    file[key]
        .as_array()
        .unwrap_or_else(|| panic!("vector key {key} is not an array"))
}

/// Decode `json` as `T`, then check that encoding it gives back exactly
/// `json` and that decoding the encoding gives the same value. Canonical
/// vectors carry no unknown fields, so a field the type drops or renames
/// shows up here.
#[cfg(test)]
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
#[cfg(test)]
pub fn refused<T: DeserializeOwned + std::fmt::Debug>(name: &str, json: &Value) {
    if let Ok(decoded) = serde_json::from_value::<T>(json.clone()) {
        panic!("{name}: decoded, expected a refusal: {decoded:?}");
    }
}
