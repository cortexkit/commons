use std::{collections::BTreeSet, path::PathBuf};

use cortexkit_llm_tools::{classify, complete, errors, extract, vectors, ValidationError};
use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

fn check_vectors<Q: DeserializeOwned, R: DeserializeOwned>(
    bytes: &str,
    input_schema: Value,
    result_schema: Value,
    request_check: fn(&Q) -> Result<(), ValidationError>,
    result_check: fn(&R) -> Result<(), ValidationError>,
) {
    let document: Value = serde_json::from_str(bytes).unwrap();
    let input = jsonschema::validator_for(&input_schema).unwrap();
    let result = jsonschema::validator_for(&result_schema).unwrap();
    for (key, schema) in [("valid_requests", &input), ("valid_results", &result)] {
        let cases = document[key].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let value = &case["value"];
            assert!(
                schema.is_valid(value),
                "{}: schema rejected {}",
                case["name"],
                value
            );
            if key == "valid_requests" {
                request_check(&serde_json::from_value::<Q>(value.clone()).unwrap()).unwrap();
            } else {
                result_check(&serde_json::from_value::<R>(value.clone()).unwrap()).unwrap();
            }
        }
    }
    for (key, schema) in [("invalid_requests", &input), ("invalid_results", &result)] {
        let cases = document[key].as_array().unwrap();
        assert!(!cases.is_empty());
        for case in cases {
            let value = &case["value"];
            assert!(
                !schema.is_valid(value),
                "{}: schema accepted {}",
                case["name"],
                value
            );
            if let Some(expected) = case.get("decode_error").and_then(Value::as_str) {
                assert_eq!(key, "invalid_requests");
                match serde_json::from_value::<Q>(value.clone()) {
                    Err(error) => assert_eq!(error.to_string(), expected, "{}", case["name"]),
                    Ok(_) => panic!("{}: expected named decode error {expected}", case["name"]),
                }
            }
            let typed_valid = if key == "invalid_requests" {
                serde_json::from_value::<Q>(value.clone()).is_ok_and(|q| request_check(&q).is_ok())
            } else {
                serde_json::from_value::<R>(value.clone()).is_ok_and(|r| result_check(&r).is_ok())
            };
            assert!(
                !typed_valid,
                "{}: typed validator accepted {}",
                case["name"], value
            );
        }
    }
}

#[test]
fn classifier_schemas_and_validators_cover_all_vectors() {
    check_vectors(
        vectors::CLASSIFY,
        classify::input_schema(),
        classify::result_schema(),
        classify::validate_request,
        classify::validate_result,
    );
}

#[test]
fn extractor_schemas_and_validators_cover_all_vectors() {
    check_vectors(
        vectors::EXTRACT,
        extract::input_schema(),
        extract::result_schema(),
        extract::validate_request,
        extract::validate_result,
    );
}

#[test]
fn completion_schemas_and_validators_cover_all_vectors() {
    check_vectors(
        vectors::COMPLETE,
        complete::input_schema(),
        complete::result_schema(),
        complete::validate_request,
        complete::validate_result,
    );
}

#[test]
fn json_results_validate_against_their_requested_schema() {
    let document: Value = serde_json::from_str(vectors::COMPLETE).unwrap();
    let mut checked_json = 0;
    for case in document["valid_results"].as_array().unwrap() {
        let index = case["request"].as_u64().unwrap() as usize;
        let req: complete::Request =
            serde_json::from_value(document["valid_requests"][index]["value"].clone()).unwrap();
        let reply: complete::Result = serde_json::from_value(case["value"].clone()).unwrap();
        complete::validate_result_for_request(&req, &reply).unwrap();
        if let Some(schema) = &req.output_schema {
            let validator = jsonschema::validator_for(&Value::Object(schema.clone())).unwrap();
            assert!(validator.is_valid(reply.json.as_ref().unwrap()));
            // A different value must fail too; this exercises the caller's schema,
            // rather than just the tool result's unrestricted json member.
            assert!(!validator.is_valid(&serde_json::json!(false)));
            checked_json += 1;
        }
    }
    assert_eq!(checked_json, 2);
}

#[test]
fn refusals_cover_every_published_code_and_class() {
    let expected: BTreeSet<_> = errors::TOOL_ADMISSION_CODES
        .iter()
        .chain(errors::TOOL_RESULT_CODES)
        .chain(errors::COMPLETE_CODES)
        .chain(errors::classifier::ADMISSION_CODES)
        .chain(errors::classifier::ITEM_CODES)
        .chain(errors::classifier::BATCH_LEVEL_CODES)
        .chain(errors::extractor::ADMISSION_CODES)
        .chain(errors::extractor::ITEM_CODES)
        .copied()
        .collect();
    let refusals: Vec<errors::Refusal> = serde_json::from_str(vectors::REFUSALS).unwrap();
    let codes: BTreeSet<_> = refusals.iter().map(|r| r.code.as_str()).collect();
    assert_eq!(codes, expected);
    assert_eq!(
        codes.len(),
        refusals.len(),
        "one vector per code, no duplicates"
    );
    for refusal in &refusals {
        assert_eq!(
            errors::class(&refusal.code),
            Some(refusal.detail.class.clone())
        );
        assert_eq!(
            errors::refusal(&refusal.code, &refusal.message)
                .unwrap()
                .detail
                .class,
            refusal.detail.class
        );
    }
    assert!(errors::class("invented_unknown_code").is_none());
    assert!(errors::refusal("invented_unknown_code", "Invented").is_none());
}

#[test]
fn vector_sha256sums_inventory_and_embedded_bytes_match() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/llm-tools-v1");
    let lines: Vec<_> = vectors::SHA256SUMS.lines().collect();
    assert_eq!(lines.len(), vectors::FILES.len());
    let mut names = BTreeSet::from(["SHA256SUMS".to_string()]);
    for (line, (name, embedded)) in lines.iter().zip(vectors::FILES) {
        let (hash, listed) = line.split_once("  ").unwrap();
        assert_eq!(listed, *name);
        assert_eq!(
            format!("{:x}", Sha256::digest(embedded.as_bytes())),
            hash,
            "{name}"
        );
        assert_eq!(std::fs::read(dir.join(name)).unwrap(), embedded.as_bytes());
        assert!(names.insert(name.to_string()));
    }
    let disk: BTreeSet<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, disk);
    assert_eq!(
        std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap(),
        vectors::SHA256SUMS
    );
}
