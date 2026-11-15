use cortexkit_usage_steps::{cause::rewrite_cause, Cause, StepRow};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const VECTORS: &str = include_str!("../test-vectors/rewrite-cause-v1/cases.json");

#[derive(Deserialize)]
struct Case {
    name: String,
    prev: Option<StepRow>,
    row: StepRow,
    cause: Option<Cause>,
}

#[test]
fn rewrite_cause_vectors() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(VECTORS).unwrap();
    assert!(cases.len() >= 50, "rewrite coverage lost cases");
    for raw in cases {
        let case: Case = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(
            rewrite_cause(case.prev.as_ref(), &case.row),
            case.cause,
            "{}",
            case.name
        );
        // Stored causes are display facts, not inputs to the explanation.
        let encoded = serde_json::to_value(&case.row).unwrap();
        assert_eq!(encoded, raw["row"], "{}", case.name);
        let decoded: StepRow = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(decoded.cause, case.row.cause, "{}", case.name);
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            encoded,
            "{}",
            case.name
        );
        assert_eq!(StepRow::check_closed(&encoded), Ok(()), "{}", case.name);
    }
}

#[test]
fn rewrite_cause_vector_checksum() {
    let sums = include_str!("../test-vectors/rewrite-cause-v1/SHA256SUMS");
    let (hash, file) = sums.trim_end().split_once("  ").unwrap();
    assert_eq!(file, "cases.json");
    assert_eq!(hash, format!("{:x}", Sha256::digest(VECTORS.as_bytes())));
}
