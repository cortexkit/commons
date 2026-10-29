use cortexkit_exec_remote_types::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

const VECTORS: &[&str] = &[
    "request-resume",
    "attached",
    "submission-never-accepted",
    "submission-key-mismatch",
];

#[derive(Debug, Serialize, Deserialize)]
struct OutcomeCase {
    request: RunRequest,
    stream: Vec<StreamRecord>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/exec-remote-v1/submission-key")
}

fn vector(name: &str) -> OutcomeCase {
    serde_json::from_slice(&fs::read(root().join(format!("{name}.jcs"))).unwrap()).unwrap()
}

fn request() -> RunRequest {
    RunRequest::new(
        "example-task",
        "/example/repo",
        "/example/repo",
        "printf hello",
    )
}

fn decode_key(key: &str) -> Result<RunRequest, serde_json::Error> {
    let mut raw = serde_json::to_value(request()).unwrap();
    raw["submission_key"] = json!(key);
    serde_json::from_value(raw)
}

#[test]
fn submission_key_length_boundaries() {
    for length in [16, 128] {
        assert!(decode_key(&"x".repeat(length)).is_ok(), "length {length}");
    }
    for length in [0, 15, 129] {
        let error = decode_key(&"x".repeat(length)).unwrap_err();
        assert!(error.to_string().contains("submission_key"));
    }
}

#[test]
fn submission_key_rejects_space() {
    let error = decode_key("example task-0001").unwrap_err();
    assert!(error.to_string().contains("submission_key"));
}

#[test]
fn submission_key_rejects_non_ascii_and_controls() {
    for key in [
        "example:task-000é",
        "example:task-000\t",
        "example:task-000\n",
        "example:task-000\0",
        "example:task-000\u{7f}",
    ] {
        assert!(decode_key(key).is_err(), "{key:?}");
    }
    assert!(decode_key("!~~~~~~~~~~~~~~~").is_ok());
}

#[test]
fn resume_from_seq_requires_submission_key() {
    for cursor in [0, u64::MAX] {
        let mut raw = serde_json::to_value(request()).unwrap();
        raw["resume_from_seq"] = json!(cursor);
        let error = serde_json::from_value::<RunRequest>(raw.clone()).unwrap_err();
        assert!(error.to_string().contains("resume_from_seq"));
        raw["submission_key"] = Value::Null;
        assert!(serde_json::from_value::<RunRequest>(raw).is_err());
    }
}

#[test]
fn submission_fields_round_trip_and_accessors() {
    assert_eq!(SUBMISSION_KEY_CAPABILITY, "exec-remote-submission-key/v1");
    let expected = request()
        .with_submission_key("example:task-0001")
        .with_resume_from_seq(7);
    assert_eq!(expected.submission_key(), Some("example:task-0001"));
    assert_eq!(expected.resume_from_seq(), Some(7));
    assert_eq!(vector("request-resume").request, expected);
    let encoded = serde_json::to_vec(&expected).unwrap();
    assert_eq!(
        serde_json::from_slice::<RunRequest>(&encoded).unwrap(),
        expected
    );
    for cursor in [0, u64::MAX] {
        let keyed = expected.clone().with_resume_from_seq(cursor);
        let raw = serde_json::to_value(&keyed).unwrap();
        assert_eq!(raw["resume_from_seq"], cursor);
        assert_eq!(serde_json::from_value::<RunRequest>(raw).unwrap(), keyed);
    }
    let mut future = serde_json::to_value(expected.clone()).unwrap();
    future["future_request_field"] = json!(true);
    assert_eq!(
        serde_json::from_value::<RunRequest>(future).unwrap(),
        expected
    );
}

#[test]
fn attached_acceptance_round_trip_and_accessor() {
    let case = vector("attached");
    let StreamRecord::Accepted(accepted) = &case.stream[0] else {
        panic!("attached vector must contain an acceptance");
    };
    let expected = Accepted::new(accepted.job_id, 1).with_attached(true);
    assert!(accepted.attached());
    assert_eq!(accepted, &expected);
    let raw = serde_json::to_value(accepted).unwrap();
    assert_eq!(raw["attached"], true);
    assert_eq!(serde_json::from_value::<Accepted>(raw).unwrap(), expected);
}

#[test]
fn submission_defaults_preserve_existing_vector_bytes() {
    let pinned = fs::read(root().join("../outcomes/exit.jcs")).unwrap();
    assert_eq!(
        format!("{:x}", Sha256::digest(&pinned)),
        "ee593b3133c6f421b447103f459c404cfb65022654d39674e20f4ca2c383db25"
    );
    let case: OutcomeCase = serde_json::from_slice(&pinned).unwrap();
    assert_eq!(case.request.submission_key(), None);
    assert_eq!(case.request.resume_from_seq(), None);
    let StreamRecord::Accepted(accepted) = &case.stream[0] else {
        panic!("exit vector must begin with acceptance");
    };
    assert!(!accepted.attached());
    assert_eq!(serde_json_canonicalizer::to_vec(&case).unwrap(), pinned);
    let mut raw = serde_json::to_value(accepted).unwrap();
    raw["attached"] = json!(false);
    let explicit_false: Accepted = serde_json::from_value(raw).unwrap();
    assert!(!explicit_false.attached());
    assert_eq!(
        serde_json::to_vec(&explicit_false).unwrap(),
        serde_json::to_vec(
            &Accepted::new(accepted.job_id, accepted.queue_position).with_attached(false)
        )
        .unwrap()
    );
    assert!(serde_json::to_value(explicit_false)
        .unwrap()
        .get("attached")
        .is_none());
}

#[test]
fn submission_never_accepted_maps_both_ways() {
    let wire = "submission_never_accepted";
    assert_eq!(
        RefusalReason::from(wire.to_owned()),
        RefusalReason::SubmissionNeverAccepted
    );
    assert_eq!(String::from(RefusalReason::SubmissionNeverAccepted), wire);
    assert_eq!(
        serde_json::to_string(&RefusalReason::SubmissionNeverAccepted).unwrap(),
        json!(wire).to_string()
    );
    assert_eq!(
        serde_json::from_value::<RefusalReason>(json!(wire)).unwrap(),
        RefusalReason::SubmissionNeverAccepted
    );
    let case = vector("submission-never-accepted");
    assert!(case.request.submission_key().is_some());
    let StreamRecord::Terminal(terminal) = &case.stream[0] else {
        panic!("never-accepted vector must contain a terminal refusal");
    };
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::SubmissionNeverAccepted
        }
    );
}

#[test]
fn older_style_refusal_decoder_preserves_new_tag_as_unknown() {
    #[derive(Debug, Deserialize, PartialEq, Eq)]
    #[serde(from = "String")]
    enum OlderRefusalReason {
        RequestRejected,
        Unknown(String),
    }
    impl From<String> for OlderRefusalReason {
        fn from(reason: String) -> Self {
            match reason.as_str() {
                "request_rejected" => Self::RequestRejected,
                _ => Self::Unknown(reason),
            }
        }
    }
    let wire = serde_json::to_string(&RefusalReason::SubmissionNeverAccepted).unwrap();
    assert_eq!(
        serde_json::from_str::<OlderRefusalReason>(&wire).unwrap(),
        OlderRefusalReason::Unknown("submission_never_accepted".into())
    );
}

#[test]
fn submission_key_mismatch_vector_is_request_rejected() {
    let case = vector("submission-key-mismatch");
    assert!(case.request.submission_key().is_some());
    let StreamRecord::Terminal(terminal) = &case.stream[0] else {
        panic!("mismatch vector must contain a terminal refusal");
    };
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::RequestRejected
        }
    );
    assert_eq!(terminal.refusal_detail(), Some("submission_key_mismatch"));
}

// The runner owns submission digests; this mirrors the contract's canonical input.
fn canonical_submission_request(mut request: Value) -> Vec<u8> {
    let fields = request.as_object_mut().unwrap();
    fields.remove("resume_from_seq");
    fields.remove("progress");
    serde_json_canonicalizer::to_vec(&request).unwrap()
}

#[test]
fn canonical_request_comparison_ignores_field_order_and_resume_cursor() {
    let first: Value = serde_json::from_str(r#"{"workspace_key":"example-task","repository_root":"/example/repo","cwd":"/example/repo","command":"printf hello","env":{},"siblings":[],"submission_key":"example:task-0001"}"#).unwrap();
    let mut resumed: Value = serde_json::from_str(r#"{"resume_from_seq":7,"submission_key":"example:task-0001","siblings":[],"env":{},"command":"printf hello","cwd":"/example/repo","repository_root":"/example/repo","workspace_key":"example-task"}"#).unwrap();
    resumed["progress"] = json!(true);
    let canonical = canonical_submission_request(first);
    assert_eq!(canonical, canonical_submission_request(resumed.clone()));
    resumed["command"] = json!("printf different");
    assert_ne!(canonical, canonical_submission_request(resumed));
}

#[test]
fn submission_request_digest_ignores_progress() {
    let original = request().with_submission_key("example:task-0001");
    let opted_in = original.clone().with_progress(true);
    let first = serde_json::to_value(original).unwrap();
    let second = serde_json::to_value(opted_in).unwrap();
    assert_ne!(first, second);
    assert_eq!(
        Sha256::digest(canonical_submission_request(first)),
        Sha256::digest(canonical_submission_request(second))
    );
}

#[test]
fn keyed_refusals_have_distinct_job_ids() {
    let attached_case = vector("attached");
    let StreamRecord::Accepted(attached) = &attached_case.stream[0] else {
        panic!("attached vector must contain an acceptance");
    };
    let mut refusal_ids = Vec::new();
    for name in ["submission-key-mismatch", "submission-never-accepted"] {
        let case = vector(name);
        let StreamRecord::Terminal(refusal) = &case.stream[0] else {
            panic!("{name} vector must contain a terminal refusal");
        };
        assert_ne!(
            refusal.job_id, attached.job_id,
            "{name} must not name the existing job"
        );
        refusal_ids.push(refusal.job_id);
    }
    assert_ne!(
        refusal_ids[0], refusal_ids[1],
        "each refusal has its own job ID"
    );
}

#[test]
fn submission_key_vector_inventory_hashes_and_round_trips() {
    let mut expected_files: BTreeSet<_> = VECTORS
        .iter()
        .flat_map(|name| [format!("{name}.jcs"), format!("{name}.sha256")])
        .collect();
    expected_files.insert("SHA256SUMS".into());
    let actual_files: BTreeSet<_> = fs::read_dir(root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(actual_files, expected_files);
    let mut sums = Vec::new();
    for name in VECTORS {
        let bytes = fs::read(root().join(format!("{name}.jcs"))).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let expected = fs::read_to_string(root().join(format!("{name}.sha256"))).unwrap();
        assert_eq!(hash, expected, "{name} SHA-256");
        sums.push((format!("{name}.jcs"), hash));
        sums.push((
            format!("{name}.sha256"),
            format!("{:x}", Sha256::digest(expected.as_bytes())),
        ));
        let case: OutcomeCase = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            serde_json_canonicalizer::to_vec(&case).unwrap(),
            bytes,
            "{name} typed RFC 8785 round-trip"
        );
    }
    sums.sort();
    let expected: String = sums
        .into_iter()
        .map(|(path, hash)| format!("{hash}  {path}\n"))
        .collect();
    assert_eq!(
        fs::read_to_string(root().join("SHA256SUMS")).unwrap(),
        expected
    );
}
