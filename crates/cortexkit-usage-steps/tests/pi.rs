use std::collections::BTreeSet;
use std::path::Path;

use cortexkit_usage_steps::{pi_step_ref, PiEntry};
use serde_json::Value;
use sha2::{Digest, Sha256};

fn cases() -> Value {
    serde_json::from_str(include_str!("../test-vectors/pi-step-ref-v1/cases.json")).unwrap()
}

fn entry(raw: &Value) -> PiEntry<'_> {
    let mut entry = PiEntry::new(raw["provider"].as_str().unwrap());
    entry.id = raw["id"].as_str();
    entry.timestamp = raw["timestamp"].as_str();
    entry.is_operation = raw["is_operation"].as_bool().unwrap_or(false);
    entry.is_assistant_message = raw["is_assistant_message"].as_bool().unwrap_or(false);
    entry.response_id = raw["response_id"].as_str();
    entry.model = raw["model"].as_str();
    entry.input = raw["input"].as_u64();
    entry.output = raw["output"].as_u64();
    entry.cache_read = raw["cache_read"].as_u64();
    entry.cache_write = raw["cache_write"].as_u64();
    entry.reasoning = raw["reasoning"].as_u64();
    entry
}

#[test]
fn pi_step_ref_matches_every_key_vector() {
    let cases = cases();
    let keys = cases["keys"].as_array().unwrap();
    assert!(keys.len() >= 19, "lost Pi key cases");
    let mut names = BTreeSet::new();
    for case in keys {
        let name = case["name"].as_str().unwrap();
        assert!(names.insert(name), "duplicate case {name}");
        assert_eq!(
            pi_step_ref(&entry(&case["entry"])).as_deref(),
            case["expected"].as_str(),
            "{name}"
        );
    }
}

#[test]
fn pi_step_ref_timestamp_vectors_match_chrono() {
    let cases = cases();
    let timestamps = cases["chrono_timestamps"].as_array().unwrap();
    assert!(timestamps.len() >= 8, "lost Chrono timestamp cases");
    let mut sample = entry(&cases["keys"][0]["entry"]);
    for case in timestamps {
        let name = case["name"].as_str().unwrap();
        let timestamp = case["timestamp"].as_str().unwrap();
        // Canonical spellings are separate inputs: this catches rejecting
        // accepted variants, rounding fractions, and ignoring numeric offsets.
        let expected_ms = case["canonical_timestamp"].as_str().map(|canonical| {
            chrono::DateTime::parse_from_rfc3339(canonical)
                .unwrap()
                .timestamp_millis()
        });
        assert_eq!(
            chrono::DateTime::parse_from_rfc3339(timestamp)
                .ok()
                .map(|timestamp| timestamp.timestamp_millis()),
            expected_ms,
            "{name}: Chrono no longer agrees with the canonical spelling"
        );
        let expected_ts = expected_ms
            .map(|timestamp| timestamp.to_string())
            .unwrap_or_else(|| "missing".to_owned());
        let expected = format!(
            "pi:message:b3aa00c0:{expected_ts}:anthropic:claude-opus-5-5:Some(2):Some(80):Some(402010):Some(370)"
        );
        sample.timestamp = Some(timestamp);
        assert_eq!(pi_step_ref(&sample), Some(expected), "{name}");
    }
}

#[test]
fn pi_step_ref_checksums_pin_every_vector_file() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/pi-step-ref-v1");
    let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
    let mut listed = BTreeSet::new();
    for line in sums.lines() {
        let (hash, file) = line.split_once("  ").unwrap();
        let bytes = std::fs::read(dir.join(file)).unwrap();
        assert_eq!(hash, format!("{:x}", Sha256::digest(bytes)), "{file}");
        assert!(listed.insert(file.to_owned()), "{file} listed twice");
    }
    let mut on_disk: BTreeSet<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|file| file.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(on_disk.remove("SHA256SUMS"));
    assert_eq!(listed, on_disk);
}
