//! The `llm-runner/v1` test vectors, compiled in.
//!
//! The files live in this crate's `test-vectors/llm-runner-v1/` and ship in
//! its package. With the `test-vectors` feature this module is public, so a
//! runner or consumer that depends on this crate by version checks exactly
//! the bytes this crate's own tests check, without a checkout of this
//! repository. [`SHA256SUMS`] pins every file's bytes; [`README`] describes
//! each file's cases and the tests that read them.

/// The vectors' directory name, inside `test-vectors/`.
pub const DIR: &str = "llm-runner-v1";

/// Every vector file as `(file name, contents)`, in the order
/// [`SHA256SUMS`] lists them.
pub const FILES: &[(&str, &str)] = &[
    (
        "admit-optional-text-refused.json",
        include_str!("../test-vectors/llm-runner-v1/admit-optional-text-refused.json"),
    ),
    (
        "admit-optional-text-timeout.json",
        include_str!("../test-vectors/llm-runner-v1/admit-optional-text-timeout.json"),
    ),
    (
        "admit-optional-text-unknown-provider.json",
        include_str!("../test-vectors/llm-runner-v1/admit-optional-text-unknown-provider.json"),
    ),
    (
        "adopt-describe.json",
        include_str!("../test-vectors/llm-runner-v1/adopt-describe.json"),
    ),
    (
        "adopt.json",
        include_str!("../test-vectors/llm-runner-v1/adopt.json"),
    ),
    (
        "baseline.json",
        include_str!("../test-vectors/llm-runner-v1/baseline.json"),
    ),
    (
        "context-ceiling.json",
        include_str!("../test-vectors/llm-runner-v1/context-ceiling.json"),
    ),
    (
        "delivery-order-describe.json",
        include_str!("../test-vectors/llm-runner-v1/delivery-order-describe.json"),
    ),
    (
        "delivery-order.json",
        include_str!("../test-vectors/llm-runner-v1/delivery-order.json"),
    ),
    (
        "errors.json",
        include_str!("../test-vectors/llm-runner-v1/errors.json"),
    ),
    (
        "head.json",
        include_str!("../test-vectors/llm-runner-v1/head.json"),
    ),
    (
        "input-token-cap-describe.json",
        include_str!("../test-vectors/llm-runner-v1/input-token-cap-describe.json"),
    ),
    (
        "input-token-cap-results.json",
        include_str!("../test-vectors/llm-runner-v1/input-token-cap-results.json"),
    ),
    (
        "input-token-cap.json",
        include_str!("../test-vectors/llm-runner-v1/input-token-cap.json"),
    ),
    (
        "plans.json",
        include_str!("../test-vectors/llm-runner-v1/plans.json"),
    ),
    (
        "read-pages.json",
        include_str!("../test-vectors/llm-runner-v1/read-pages.json"),
    ),
    (
        "read-requests.json",
        include_str!("../test-vectors/llm-runner-v1/read-requests.json"),
    ),
    (
        "refuse-later-plan-after-planless-first-episode.json",
        include_str!(
            "../test-vectors/llm-runner-v1/refuse-later-plan-after-planless-first-episode.json"
        ),
    ),
    (
        "refuse-later-send-with-different-plan.json",
        include_str!("../test-vectors/llm-runner-v1/refuse-later-send-with-different-plan.json"),
    ),
    (
        "refuse-tool-name-collision.json",
        include_str!("../test-vectors/llm-runner-v1/refuse-tool-name-collision.json"),
    ),
    (
        "reply-too-large.json",
        include_str!("../test-vectors/llm-runner-v1/reply-too-large.json"),
    ),
    (
        "role-describe.json",
        include_str!("../test-vectors/llm-runner-v1/role-describe.json"),
    ),
    (
        "run-cancel-describe.json",
        include_str!("../test-vectors/llm-runner-v1/run-cancel-describe.json"),
    ),
    (
        "run-cancel.json",
        include_str!("../test-vectors/llm-runner-v1/run-cancel.json"),
    ),
    (
        "run-parameters-describe.json",
        include_str!("../test-vectors/llm-runner-v1/run-parameters-describe.json"),
    ),
    (
        "run-parameters.json",
        include_str!("../test-vectors/llm-runner-v1/run-parameters.json"),
    ),
    (
        "run-result.json",
        include_str!("../test-vectors/llm-runner-v1/run-result.json"),
    ),
    (
        "run-tool-calls-describe.json",
        include_str!("../test-vectors/llm-runner-v1/run-tool-calls-describe.json"),
    ),
    (
        "run-tool-calls.json",
        include_str!("../test-vectors/llm-runner-v1/run-tool-calls.json"),
    ),
    (
        "run_status.usage_model_switch.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_model_switch.json"),
    ),
    (
        "run_status.usage_plain.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_plain.json"),
    ),
    (
        "run_status.usage_reasoning.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_reasoning.json"),
    ),
    (
        "run_status.usage_unreported_step.json",
        include_str!("../test-vectors/llm-runner-v1/run_status.usage_unreported_step.json"),
    ),
    (
        "send-status-describe.json",
        include_str!("../test-vectors/llm-runner-v1/send-status-describe.json"),
    ),
    (
        "send-status.json",
        include_str!("../test-vectors/llm-runner-v1/send-status.json"),
    ),
    (
        "send.json",
        include_str!("../test-vectors/llm-runner-v1/send.json"),
    ),
    (
        "serializer-anthropic-messages.json",
        include_str!("../test-vectors/llm-runner-v1/serializer-anthropic-messages.json"),
    ),
    (
        "session-change.json",
        include_str!("../test-vectors/llm-runner-v1/session-change.json"),
    ),
    (
        "session-ref.json",
        include_str!("../test-vectors/llm-runner-v1/session-ref.json"),
    ),
    (
        "structured-output-describe.json",
        include_str!("../test-vectors/llm-runner-v1/structured-output-describe.json"),
    ),
    (
        "structured-output-results.json",
        include_str!("../test-vectors/llm-runner-v1/structured-output-results.json"),
    ),
    (
        "structured-output.json",
        include_str!("../test-vectors/llm-runner-v1/structured-output.json"),
    ),
    (
        "subscribe.json",
        include_str!("../test-vectors/llm-runner-v1/subscribe.json"),
    ),
    (
        "tags-describe.json",
        include_str!("../test-vectors/llm-runner-v1/tags-describe.json"),
    ),
    (
        "tags.json",
        include_str!("../test-vectors/llm-runner-v1/tags.json"),
    ),
    (
        "README.md",
        include_str!("../test-vectors/llm-runner-v1/README.md"),
    ),
];

/// `sha256sum` output over [`FILES`]: one `<hex>  <name>` line per file.
pub const SHA256SUMS: &str = include_str!("../test-vectors/llm-runner-v1/SHA256SUMS");

/// The vectors' `README.md`.
pub const README: &str = include_str!("../test-vectors/llm-runner-v1/README.md");

/// The contents of the vector file `name`, if it is one of [`FILES`].
pub fn file(name: &str) -> Option<&'static str> {
    FILES
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, contents)| *contents)
}

/// Test helpers over the vectors, for this crate's own tests.
#[cfg(test)]
pub(crate) use test_support::*;

#[cfg(test)]
mod test_support {
    use serde::{de::DeserializeOwned, Serialize};
    use serde_json::Value;

    use super::file;

    /// The vector file `name`, decoded.
    pub fn load(name: &str) -> Value {
        let text = file(name).unwrap_or_else(|| panic!("{name} is not listed in FILES"));
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// Re-encode a typed object inside an unchanged upstream fixture envelope.
    /// Compare actual bytes, including field order and indentation, rather than
    /// normalizing through `Value`, which would hide order and whitespace changes.
    pub fn exact_object_round_trip<T>(name: &str, marker: &str, indent: usize) -> T
    where
        T: DeserializeOwned + Serialize,
    {
        let bytes = file(name)
            .unwrap_or_else(|| panic!("{name} is not listed in FILES"))
            .to_owned();
        let start = bytes.find(marker).unwrap() + marker.len();
        let mut stream = serde_json::Deserializer::from_str(&bytes[start..]).into_iter::<Value>();
        stream.next().unwrap().unwrap();
        let end = start + stream.byte_offset();
        let decoded: T = serde_json::from_str(&bytes[start..end]).unwrap();
        let encoded = serde_json::to_string_pretty(&decoded)
            .unwrap()
            .replace('\n', &format!("\n{}", " ".repeat(indent)));
        let mut rebuilt = bytes.clone();
        rebuilt.replace_range(start..end, &encoded);
        assert_eq!(rebuilt.as_bytes(), bytes.as_bytes(), "{name}: bytes differ");
        decoded
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
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use sha2::{Digest, Sha256};

    use super::*;

    fn dir() -> String {
        format!("{}/test-vectors/{DIR}", env!("CARGO_MANIFEST_DIR"))
    }

    /// The compiled-in bytes are exactly the files on disk, and every file
    /// in the directory is listed, so a new file cannot ship unread.
    #[test]
    fn every_vector_file_on_disk_is_listed_with_its_bytes() {
        let dir = dir();
        let mut on_disk: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name != "SHA256SUMS")
            .collect();
        on_disk.sort();
        let mut listed: Vec<String> = FILES.iter().map(|(name, _)| name.to_string()).collect();
        listed.sort();
        assert_eq!(on_disk, listed);
        for (name, contents) in FILES {
            let bytes = std::fs::read_to_string(format!("{dir}/{name}")).unwrap();
            assert_eq!(*contents, bytes, "{name}");
            assert_eq!(file(name), Some(*contents));
        }
        assert_eq!(file("no-such-file.json"), None);
        assert_eq!(Some(README), file("README.md"));
        let sums = std::fs::read_to_string(format!("{dir}/SHA256SUMS")).unwrap();
        assert_eq!(SHA256SUMS, sums);
    }

    /// `SHA256SUMS` names every file once, in `FILES` order, with the
    /// SHA-256 of its bytes.
    #[test]
    fn files_match_sha256sums() {
        let names: Vec<&str> = SHA256SUMS
            .lines()
            .map(|line| line.split_once("  ").expect("sha256sum line").1)
            .collect();
        let in_order: Vec<&str> = FILES.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, in_order, "SHA256SUMS lists FILES in order");
        let checksums: BTreeMap<&str, &str> = SHA256SUMS
            .lines()
            .map(|line| {
                let (digest, name) = line.split_once("  ").unwrap();
                (name, digest)
            })
            .collect();
        assert_eq!(
            checksums.keys().copied().collect::<BTreeSet<_>>(),
            in_order.iter().copied().collect::<BTreeSet<_>>()
        );
        for (name, contents) in FILES {
            assert_eq!(
                format!("{:x}", Sha256::digest(contents.as_bytes())),
                checksums[name],
                "{name}: SHA-256 mismatch"
            );
        }
    }
}
