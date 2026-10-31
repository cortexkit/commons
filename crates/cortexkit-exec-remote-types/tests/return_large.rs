use cortexkit_exec_remote_types::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

const CAP_VECTORS: &[&str] = &[
    "v2-refuse-over-cap",
    "v3-accept-40-mib",
    "v3-accept-ceiling",
    "v3-refuse-over-ceiling",
    "v3-refuse-runner-limit",
    "v3-accept-runner-limit",
    "v3-default-chunked",
    "v3-default-legacy",
    "v3-refuse-large-legacy",
    "v2-default-chunked",
    "v2-accept-explicit-limit",
];
const REPLAY_VECTORS: &[&str] = &["replay-resume", "replay-expired-resume"];
const OLDER_VECTOR: &str = "older-refuse-over-cap";

#[derive(Debug, Serialize, Deserialize)]
struct CapCase {
    capabilities: Vec<String>,
    runner_max: u64,
    request: RunRequest,
    expect: CapExpectation,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum CapExpectation {
    Accepted { cap_bytes: u64 },
    Refused { error: ErrorReply },
}

#[derive(Debug, Serialize, Deserialize)]
struct ErrorReply {
    code: String,
    detail: ReturnCapRefusal,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReplayCase {
    capabilities: Vec<String>,
    runner_max: u64,
    request: RunRequest,
    records: Vec<StreamRecord>,
    replay: Vec<StreamRecord>,
    expect: ReplayExpectation,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReplayExpectation {
    data: BytePayload,
    expired: bool,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/exec-remote-v1/return-large")
}

fn bytes(name: &str) -> Vec<u8> {
    fs::read(root().join(format!("{name}.jcs"))).unwrap()
}

fn vector<T: DeserializeOwned>(name: &str) -> T {
    serde_json::from_slice(&bytes(name)).unwrap()
}

fn request() -> RunRequest {
    RunRequest::new(
        "example-task",
        "/example/repo",
        "/example/repo",
        "printf hello",
    )
    .with_return_paths(vec!["dist/app.bin".into()])
}

#[test]
fn large_return_constants_preserve_existing_caps_and_chunk_size() {
    assert_eq!(RETURN_LARGE_CAPABILITY, "exec-remote-return/v3");
    assert_eq!(RETURN_LARGE_MAX_BYTES_CAP, 134_217_728);
    assert_eq!(RETURN_CHUNKED_CAPABILITY, "exec-remote-return/v2");
    assert_eq!(RETURN_CHUNKED_MAX_BYTES_CAP, 10_485_760);
    assert_eq!(RETURN_MAX_BYTES_CAP, 262_144);
    assert_eq!(RETURN_CHUNK_MAX_BYTES, 524_288);
}

#[test]
fn cap_vectors_match_admission_or_numeric_invalid_params_detail() {
    for name in CAP_VECTORS {
        let case: CapCase = vector(name);
        if case.runner_max > 10_485_760 {
            assert!(case
                .capabilities
                .iter()
                .any(|cap| cap == RETURN_LARGE_CAPABILITY));
        }
        assert!(case
            .capabilities
            .iter()
            .any(|cap| cap == RETURN_CHUNKED_CAPABILITY));
        let result = check_return_cap(&case.request, case.runner_max);
        match case.expect {
            CapExpectation::Accepted { cap_bytes } => assert_eq!(result, Ok(cap_bytes), "{name}"),
            CapExpectation::Refused { error } => {
                assert_eq!(error.code, "invalid_params", "{name}");
                assert_eq!(error.detail.field(), "return_max_bytes", "{name}");
                let max_bytes = error
                    .detail
                    .max_bytes()
                    .expect("new runners state their limit");
                let refusal = result.unwrap_err();
                assert_eq!(refusal, error.detail, "{name}");
                assert_eq!(
                    serde_json::to_value(refusal).unwrap(),
                    json!({
                        "field":"return_max_bytes", "max_bytes": max_bytes
                    }),
                    "{name}"
                );
                let raw: Value = vector(name);
                assert!(
                    raw["expect"]["error"]["detail"]["max_bytes"].is_u64(),
                    "{name}"
                );
                assert!(raw["expect"]["error"].get("job_id").is_none(), "{name}");
            }
        }
    }
}

#[test]
fn older_peer_refusal_without_limit_remains_readable() {
    let case: CapCase = vector(OLDER_VECTOR);
    let CapExpectation::Refused { error } = case.expect else {
        panic!("older peer must refuse the over-cap request");
    };
    assert_eq!(error.code, "invalid_params");
    assert_eq!(error.detail.field(), "return_max_bytes");
    assert_eq!(error.detail.max_bytes(), None);
    assert_eq!(
        serde_json::to_value(error.detail).unwrap(),
        json!({"field":"return_max_bytes"})
    );
    let new_refusal = check_return_cap(&case.request, case.runner_max).unwrap_err();
    assert_eq!(new_refusal.max_bytes(), Some(10_485_760));
}

#[test]
fn cap_check_never_clamps_explicit_values_or_defaults() {
    let chunked = request().with_return_chunked(true);
    assert_eq!(check_return_cap(&chunked, 134_217_728), Ok(10_485_760));
    assert_eq!(check_return_cap(&request(), 134_217_728), Ok(262_144));
    for (request, runner_max, expected_max) in [
        (chunked.clone(), 10_485_759, 10_485_759),
        (request(), 262_143, 262_143),
        (
            chunked.clone().with_return_max_bytes(134_217_729),
            u64::MAX,
            134_217_728,
        ),
        (
            chunked.clone().with_return_max_bytes(u64::MAX),
            u64::MAX,
            134_217_728,
        ),
        (
            request().with_return_max_bytes(262_145),
            134_217_728,
            262_144,
        ),
    ] {
        assert_eq!(
            check_return_cap(&request, runner_max),
            Err(ReturnCapRefusal::new(expected_max))
        );
    }
    assert_eq!(
        check_return_cap(&chunked.clone().with_return_max_bytes(0), 0),
        Ok(0)
    );
    assert_eq!(
        check_return_cap(&chunked.with_return_max_bytes(1), 1),
        Ok(1)
    );
    assert_eq!(
        check_return_cap(&request().with_return_max_bytes(262_144), 134_217_728),
        Ok(262_144)
    );
}

#[test]
fn cap_refusal_detail_is_additive_and_max_bytes_is_numeric() {
    let detail: ReturnCapRefusal = serde_json::from_value(json!({
        "field":"return_max_bytes", "max_bytes":67_108_864, "future_field":true
    }))
    .unwrap();
    assert_eq!(detail, ReturnCapRefusal::new(67_108_864));
    assert_eq!(
        serde_json::to_value(detail).unwrap(),
        json!({
            "field":"return_max_bytes", "max_bytes":67_108_864
        })
    );
    for invalid in [json!("67108864"), json!(-1), json!(1.5), json!(true)] {
        assert!(serde_json::from_value::<ReturnCapRefusal>(json!({
            "field":"return_max_bytes", "max_bytes":invalid
        }))
        .is_err());
    }
    assert!(serde_json::from_value::<ReturnCapRefusal>(json!({"max_bytes":10})).is_err());
}

#[test]
fn retained_large_return_replays_chunks_and_manifest_after_resume_cursor() {
    check_replay("replay-resume", false);
}

#[test]
fn expired_large_return_replays_existing_expired_manifest_after_resume_cursor() {
    check_replay("replay-expired-resume", true);
}

fn check_replay(name: &str, expired: bool) {
    let case: ReplayCase = vector(name);
    assert_eq!(case.expect.expired, expired);
    assert!(case
        .capabilities
        .iter()
        .any(|cap| cap == RETURN_LARGE_CAPABILITY));
    assert!(case
        .capabilities
        .iter()
        .any(|cap| cap == SUBMISSION_KEY_CAPABILITY));
    assert_eq!(case.request.submission_key(), Some("example:task-0001"));
    let cursor = case.request.resume_from_seq().unwrap();
    assert_eq!(cursor, 4);
    let cap = check_return_cap(&case.request, case.runner_max).unwrap();
    assert_eq!(cap, 41_943_040);
    let StreamRecord::Accepted(accepted) = &case.replay[0] else {
        panic!("reattachment must report the existing job");
    };
    assert!(accepted.attached());
    let StreamRecord::Output(last_output) = &case.records[0] else {
        panic!("original stream must begin with the last output before chunks");
    };
    assert_eq!(last_output.seq, cursor);
    let mut original = ReturnAssembler::new(case.request.return_paths(), cap);
    for record in &case.records[1..] {
        match record {
            StreamRecord::ReturnChunk(chunk) => {
                original.accept_chunk(chunk).unwrap();
            }
            StreamRecord::ReturnManifest(manifest) => {
                assert_eq!(manifest.cap_bytes(), cap);
                assert!(original.finish(manifest).unwrap()[0].is_returned());
                break;
            }
            other => panic!("unexpected original record: {other:?}"),
        }
    }
    if !expired {
        assert_eq!(
            case.replay[1..],
            case.records[1..],
            "replay must preserve records and sequence numbers"
        );
    }
    let mut assembler = ReturnAssembler::new(case.request.return_paths(), cap);
    let mut received = Vec::new();
    let mut replay_manifest = None;
    for record in &case.replay[1..] {
        match record {
            StreamRecord::ReturnChunk(chunk) => {
                assert!(!expired);
                assert!(chunk.seq() > cursor);
                assert_eq!(chunk.job_id(), accepted.job_id);
                received.extend_from_slice(assembler.accept_chunk(chunk).unwrap());
            }
            StreamRecord::ReturnManifest(manifest) => {
                assert!(manifest.seq() > cursor);
                assert_eq!(manifest.job_id(), accepted.job_id);
                assert!(replay_manifest.replace(manifest).is_none());
            }
            StreamRecord::Terminal(terminal) => {
                assert_eq!(terminal.job_id, accepted.job_id);
                assert_eq!(terminal.outcome, Outcome::Exit { code: 0 });
            }
            other => panic!("unexpected replay record: {other:?}"),
        }
    }
    assert_eq!(received, case.expect.data.0);
    let manifest = replay_manifest.expect("replay must contain a manifest");
    assert_eq!(manifest.cap_bytes(), cap);
    assert_eq!(manifest.seq(), 7);
    assert_eq!(manifest.files()[0].chunks(), Some(2));
    assert_eq!(manifest.files()[0].size(), Some(5));
    let verified = if expired {
        assert_eq!(manifest.total_bytes(), 0);
        assembler
            .finish_expired_replay(manifest, last_output.seq)
            .unwrap()
    } else {
        assert_eq!(received, b"hello");
        assert_eq!(manifest.total_bytes(), 5);
        assembler.finish(manifest).unwrap()
    };
    assert_eq!(verified.len(), 1);
    assert_eq!(verified[0].path(), "dist/hello.txt");
    if expired {
        assert!(matches!(
            verified[0].outcome(),
            ReturnManifestOutcome::Failed {
                cause: ReturnPathsFailureCause::Expired,
                ..
            }
        ));
        assert_eq!(
            case.replay.last(),
            case.records.last(),
            "expired returned bytes do not change the job outcome"
        );
    } else {
        assert!(verified[0].is_returned());
    }
}

#[test]
fn every_large_return_vector_round_trips_and_has_pinned_checksums() {
    let names: Vec<_> = CAP_VECTORS
        .iter()
        .chain(REPLAY_VECTORS)
        .copied()
        .chain([OLDER_VECTOR])
        .collect();
    let mut expected_files: BTreeSet<_> = names
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
    for name in names {
        let bytes = bytes(name);
        let canonical = if REPLAY_VECTORS.contains(&name) {
            serde_json_canonicalizer::to_vec(&vector::<ReplayCase>(name)).unwrap()
        } else {
            serde_json_canonicalizer::to_vec(&vector::<CapCase>(name)).unwrap()
        };
        assert_eq!(canonical, bytes, "{name} typed canonical round-trip");
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let expected = fs::read_to_string(root().join(format!("{name}.sha256"))).unwrap();
        assert_eq!(hash, expected, "{name} SHA-256");
        sums.push((format!("{name}.jcs"), hash));
        sums.push((
            format!("{name}.sha256"),
            format!("{:x}", Sha256::digest(expected.as_bytes())),
        ));
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
