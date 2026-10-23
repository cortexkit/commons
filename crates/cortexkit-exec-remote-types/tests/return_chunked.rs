//! Chunked returned files: the `return_chunked` request flag, the
//! `return_chunk` and `return_manifest` records, and `ReturnAssembler`, checked
//! against the vectors in `test-vectors/exec-remote-v1/return-chunked/`.

use base64::{engine::general_purpose::STANDARD, Engine};
use cortexkit_exec_remote_types::*;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

const VECTORS: &[&str] = &[
    "two-files-one-empty",
    "three-chunks-short-last",
    "failed-missing",
    "failed-not-regular",
    "failed-escape",
    "failed-over-cap",
    "read-error-after-chunks",
    "replay-retained",
    "replay-expired",
    "replay-retained-counted",
    "replay-expired-counted",
    "read-error-after-chunks-counted",
    "unknown-cause-discards-chunks",
    "refuse-offset-gap",
    "refuse-offset-overlap",
    "refuse-interleaved-paths",
    "refuse-chunk-too-large",
    "refuse-empty-chunk",
    "refuse-path-index-out-of-range",
    "refuse-digest-mismatch",
    "refuse-size-mismatch",
    "refuse-path-not-echoed",
    "refuse-chunks-before-over-cap",
    "refuse-total-over-cap",
    "refuse-chunk-count-mismatch",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/exec-remote-v1/return-chunked")
}

fn bytes(name: &str) -> Vec<u8> {
    fs::read(root().join(format!("{name}.jcs"))).unwrap()
}

fn vector(name: &str) -> Value {
    serde_json::from_slice(&bytes(name)).unwrap()
}

fn job_id() -> Uuid {
    "0192a64a-1234-7000-8000-000000000001".parse().unwrap()
}

fn records(name: &str) -> Vec<StreamRecord> {
    serde_json::from_value(vector(name)["records"].clone()).unwrap()
}

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

#[test]
fn return_chunked_vector_inventory_is_complete() {
    let mut expected: BTreeSet<_> = VECTORS
        .iter()
        .flat_map(|name| [format!("{name}.jcs"), format!("{name}.sha256")])
        .collect();
    expected.insert("SHA256SUMS".to_owned());
    let actual: BTreeSet<_> = fs::read_dir(root())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(
        actual, expected,
        "missing or unchecked return-chunked cases"
    );
}

#[test]
fn return_chunked_vector_sha256_matches_jcs_bytes() {
    let mut sums = Vec::new();
    for name in VECTORS {
        // Hash the pinned file, not a re-encoding that could mask changed bytes.
        let hash = sha256_hex(&bytes(name));
        let expected = fs::read_to_string(root().join(format!("{name}.sha256"))).unwrap();
        assert_eq!(expected, hash, "{name} SHA-256");
        sums.push((format!("{name}.jcs"), hash));
        sums.push((format!("{name}.sha256"), sha256_hex(expected.as_bytes())));
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

#[test]
fn return_chunked_vectors_are_canonical_and_records_round_trip_byte_for_byte() {
    for name in VECTORS {
        let pinned = bytes(name);
        let value: Value = serde_json::from_slice(&pinned).unwrap();
        assert_eq!(
            serde_json_canonicalizer::to_vec(&value).unwrap(),
            pinned,
            "{name} is RFC 8785 canonical"
        );
        for raw in value["records"].as_array().unwrap() {
            let record: StreamRecord = serde_json::from_value(raw.clone()).unwrap();
            assert!(
                !matches!(record, StreamRecord::Unknown { .. }),
                "{name}: {raw} must decode as a known record"
            );
            assert_eq!(
                serde_json_canonicalizer::to_vec(&record).unwrap(),
                serde_json_canonicalizer::to_vec(raw).unwrap(),
                "{name}: {raw} round-trips through StreamRecord"
            );
        }
    }
}

/// The assembler's result in the vector's `expect` shape: either the verified
/// files with the bytes accepted for each returned path, or the first error.
fn assemble(name: &str) -> Value {
    let case = vector(name);
    let paths: Vec<String> = serde_json::from_value(case["return_paths"].clone()).unwrap();
    let cap = case["cap_bytes"].as_u64().unwrap();
    let mut assembler = ReturnAssembler::new(&paths, cap);
    // What a caller would write, per path, from the bytes accept_chunk yields.
    // Like a caller, this writes each chunk at its offset and trusts the
    // assembler to have refused gaps and overlaps.
    let mut written = vec![Vec::<u8>::new(); paths.len()];
    for record in records(name) {
        match record {
            StreamRecord::ReturnChunk(chunk) => match assembler.accept_chunk(&chunk) {
                Ok(data) => {
                    let file = &mut written[chunk.path_index() as usize];
                    let start = chunk.offset() as usize;
                    if file.len() < start + data.len() {
                        file.resize(start + data.len(), 0);
                    }
                    file[start..start + data.len()].copy_from_slice(data);
                }
                Err(error) => return error_value(&error),
            },
            StreamRecord::ReturnManifest(manifest) => {
                let result = match case.get("last_output_seq").and_then(Value::as_u64) {
                    Some(seq) => assembler.finish_expired_replay(&manifest, seq),
                    None => assembler.finish(&manifest),
                };
                return match result {
                    Ok(verified) => json!({"files": verified
                        .iter()
                        .map(|file| verified_value(file, &written))
                        .collect::<Vec<_>>()}),
                    Err(error) => error_value(&error),
                };
            }
            _ => {}
        }
    }
    panic!("{name}: no return_manifest")
}

fn error_value(error: &ReturnStreamError) -> Value {
    let mut value = json!({"code": error.code(), "path_index": error.path_index()});
    if let Some(offset) = error.offset() {
        value["offset"] = json!(offset);
    }
    json!({ "error": value })
}

fn verified_value(file: &VerifiedReturn, written: &[Vec<u8>]) -> Value {
    let mut value = json!({"path_index": file.path_index(), "path": file.path()});
    match file.outcome() {
        ReturnManifestOutcome::Returned { size, sha256 } => {
            value["size"] = json!(size);
            value["sha256"] = json!(sha256);
            value["data"] = json!(STANDARD.encode(&written[file.path_index() as usize]));
        }
        ReturnManifestOutcome::Failed { cause, size } => {
            value["cause"] = json!(cause);
            if let Some(size) = size {
                value["size"] = json!(size);
            }
        }
        other => panic!("unexpected outcome {other:?}"),
    }
    value
}

fn check_vector(name: &str) {
    assert_eq!(assemble(name), vector(name)["expect"], "{name}");
    let records = records(name);
    assert!(
        !records
            .iter()
            .any(|record| matches!(record, StreamRecord::ReturnResult(_))),
        "{name}: a chunked stream never carries return_result"
    );
    let [.., StreamRecord::ReturnManifest(_), StreamRecord::Terminal(_)] = records.as_slice()
    else {
        panic!("{name}: the manifest must immediately precede the terminal record")
    };
}

macro_rules! vector_tests {
    ($($test:ident => $name:literal,)*) => {
        $(
            #[test]
            fn $test() {
                check_vector($name);
            }
        )*

        #[test]
        fn every_vector_has_a_named_assembler_test() {
            let named: BTreeSet<&str> = [$($name),*].into_iter().collect();
            assert_eq!(named, VECTORS.iter().copied().collect::<BTreeSet<_>>());
        }
    };
}

vector_tests! {
    vector_two_files_one_empty => "two-files-one-empty",
    vector_three_chunks_short_last => "three-chunks-short-last",
    vector_failed_missing => "failed-missing",
    vector_failed_not_regular => "failed-not-regular",
    vector_failed_escape => "failed-escape",
    vector_failed_over_cap => "failed-over-cap",
    vector_read_error_after_chunks => "read-error-after-chunks",
    vector_replay_retained => "replay-retained",
    vector_replay_expired => "replay-expired",
    vector_replay_retained_counted => "replay-retained-counted",
    vector_replay_expired_counted => "replay-expired-counted",
    vector_read_error_after_chunks_counted => "read-error-after-chunks-counted",
    vector_unknown_cause_discards_chunks => "unknown-cause-discards-chunks",
    refuses_offset_gap => "refuse-offset-gap",
    refuses_offset_overlap => "refuse-offset-overlap",
    refuses_interleaved_paths => "refuse-interleaved-paths",
    refuses_chunk_too_large => "refuse-chunk-too-large",
    refuses_empty_chunk => "refuse-empty-chunk",
    refuses_path_index_out_of_range => "refuse-path-index-out-of-range",
    refuses_digest_mismatch => "refuse-digest-mismatch",
    refuses_size_mismatch => "refuse-size-mismatch",
    refuses_path_not_echoed => "refuse-path-not-echoed",
    refuses_chunks_before_over_cap => "refuse-chunks-before-over-cap",
    refuses_total_over_cap => "refuse-total-over-cap",
    refuses_chunk_count_mismatch => "refuse-chunk-count-mismatch",
}

#[test]
fn every_failure_cause_has_a_vector() {
    let mut causes = BTreeSet::new();
    for name in VECTORS.iter().filter(|name| !name.starts_with("refuse-")) {
        for record in records(name) {
            if let StreamRecord::ReturnManifest(manifest) = record {
                causes.extend(
                    manifest
                        .files()
                        .iter()
                        .filter_map(|entry| entry.cause().cloned().map(String::from)),
                );
            }
        }
    }
    // Every known cause that can occur in chunked mode (frame_limit cannot),
    // plus one unrecognised cause.
    let expected: BTreeSet<String> = [
        ReturnPathsFailureCause::Missing,
        ReturnPathsFailureCause::NotRegular,
        ReturnPathsFailureCause::Escape,
        ReturnPathsFailureCause::OverCap,
        ReturnPathsFailureCause::ReadError,
        ReturnPathsFailureCause::Expired,
        ReturnPathsFailureCause::Unknown("future_cause".into()),
    ]
    .into_iter()
    .map(String::from)
    .collect();
    assert_eq!(causes, expected);
}

#[test]
fn replay_after_expiry_keeps_seq_sizes_and_failed_causes() {
    let retained = records("replay-retained");
    let expired = records("replay-expired");
    let manifest = |records: &[StreamRecord]| {
        records
            .iter()
            .find_map(|record| match record {
                StreamRecord::ReturnManifest(manifest) => Some(manifest.clone()),
                _ => None,
            })
            .unwrap()
    };
    let (original, replayed) = (manifest(&retained), manifest(&expired));
    assert_eq!(replayed.seq(), original.seq(), "the manifest keeps its seq");
    assert_eq!(replayed.seq(), 6);
    assert_eq!(replayed.total_bytes(), 0);
    assert_eq!(replayed.cap_bytes(), original.cap_bytes());
    for (before, after) in original.files().iter().zip(replayed.files()) {
        assert_eq!(after.path(), before.path());
        assert_eq!(after.size(), before.size(), "{}", before.path());
        match before.outcome() {
            ReturnManifestOutcome::Returned { .. } => {
                assert_eq!(after.cause(), Some(&ReturnPathsFailureCause::Expired))
            }
            _ => assert_eq!(after.outcome(), before.outcome()),
        }
    }
    // The expired replay is the retained stream without its chunk records, so
    // its only seq gap sits where the chunks were: after output seq 2 and
    // before the manifest at seq 6.
    let without_chunks: Vec<_> = retained
        .iter()
        .filter(|record| !matches!(record, StreamRecord::ReturnChunk(_)))
        .filter(|record| !matches!(record, StreamRecord::ReturnManifest(_)))
        .cloned()
        .collect();
    let mut expired_rest = expired.clone();
    expired_rest.retain(|record| !matches!(record, StreamRecord::ReturnManifest(_)));
    assert_eq!(expired_rest, without_chunks);
    let chunk_seqs: Vec<u64> = retained
        .iter()
        .filter_map(|record| match record {
            StreamRecord::ReturnChunk(chunk) => Some(chunk.seq()),
            _ => None,
        })
        .collect();
    assert_eq!(chunk_seqs, [3, 4, 5]);
    let seqs: Vec<u64> = expired
        .iter()
        .filter_map(|record| match record {
            StreamRecord::Started(started) => Some(started.seq()),
            StreamRecord::Output(output) => Some(output.seq),
            StreamRecord::ReturnManifest(manifest) => Some(manifest.seq()),
            _ => None,
        })
        .collect();
    assert_eq!(seqs, [1, 2, 6]);
}

#[test]
fn return_chunked_request_field_is_optional_and_omitted_when_false() {
    assert_eq!(RETURN_CHUNKED_CAPABILITY, "exec-remote-return/v2");
    assert_eq!(RETURN_CHUNKED_MAX_BYTES_CAP, 10_485_760);
    assert_eq!(RETURN_CHUNK_MAX_BYTES, 524_288);
    assert_eq!(RETURN_MAX_BYTES_CAP, 262_144);
    let legacy = RunRequest::new("/workspace", "/repo", "/workspace", "true")
        .with_return_paths(vec!["dist/a.txt".into()]);
    assert!(!legacy.return_chunked());
    let legacy_wire = serde_json::to_value(&legacy).unwrap();
    assert!(legacy_wire.get("return_chunked").is_none());
    assert!(!serde_json::from_value::<RunRequest>(legacy_wire)
        .unwrap()
        .return_chunked());
    let chunked = legacy.clone().with_return_chunked(true);
    assert!(chunked.return_chunked());
    assert_eq!(
        serde_json::to_value(&chunked).unwrap(),
        json!({
            "workspace_key": "/workspace", "repository_root": "/repo", "cwd": "/workspace",
            "command": "true", "env": {}, "siblings": [],
            "return_paths": ["dist/a.txt"], "return_chunked": true
        })
    );
    let decoded: RunRequest =
        serde_json::from_value(serde_json::to_value(&chunked).unwrap()).unwrap();
    assert_eq!(decoded, chunked);
    assert_eq!(
        serde_json::to_value(chunked.with_return_chunked(false)).unwrap(),
        serde_json::to_value(legacy).unwrap()
    );
}

#[test]
fn chunked_failure_causes_have_known_wire_names() {
    for (wire, cause) in [
        ("read_error", ReturnPathsFailureCause::ReadError),
        ("expired", ReturnPathsFailureCause::Expired),
        ("over_cap", ReturnPathsFailureCause::OverCap),
        (
            "future_cause",
            ReturnPathsFailureCause::Unknown("future_cause".into()),
        ),
    ] {
        assert_eq!(ReturnPathsFailureCause::from(wire.to_owned()), cause);
        assert_eq!(String::from(cause.clone()), wire);
        assert_eq!(serde_json::to_value(&cause).unwrap(), json!(wire));
    }
}

#[test]
fn manifest_entry_requires_exactly_one_of_sha256_and_cause() {
    let digest = sha256_hex(b"");
    for malformed in [
        json!({"path": "a", "size": 0, "sha256": digest, "cause": "missing"}),
        json!({"path": "a", "size": 0}),
        json!({"path": "a"}),
        json!({"path": "a", "sha256": digest}),
    ] {
        assert!(
            serde_json::from_value::<ReturnManifestEntry>(malformed.clone()).is_err(),
            "{malformed}"
        );
        let record = json!({
            "type": "return_manifest", "job_id": job_id(), "seq": 2,
            "files": [malformed], "total_bytes": 0, "cap_bytes": 10
        });
        assert!(
            serde_json::from_value::<StreamRecord>(record.clone()).is_err(),
            "a malformed entry must not turn {record} into an unknown record"
        );
    }
    let failed: ReturnManifestEntry =
        serde_json::from_value(json!({"path": "a", "cause": "missing"})).unwrap();
    assert_eq!(
        failed,
        ReturnManifestEntry::failed("a", ReturnPathsFailureCause::Missing)
    );
    assert_eq!(failed.size(), None);
    assert_eq!(failed.sha256(), None);
    let sized: ReturnManifestEntry =
        serde_json::from_value(json!({"path": "a", "cause": "over_cap", "size": 9})).unwrap();
    assert_eq!(
        sized,
        ReturnManifestEntry::failed("a", ReturnPathsFailureCause::OverCap).with_size(9)
    );
    assert_eq!(sized.size(), Some(9));
    let returned: ReturnManifestEntry =
        serde_json::from_value(json!({"path": "a", "size": 0, "sha256": digest})).unwrap();
    assert_eq!(
        returned,
        ReturnManifestEntry::returned("a", 0, digest.clone())
    );
    assert_eq!(returned.sha256(), Some(digest.as_str()));
    assert_eq!(returned.cause(), None);
    assert_eq!(returned.size(), Some(0));
}

#[test]
fn manifest_chunks_are_omitted_when_absent() {
    for entry in [
        ReturnManifestEntry::returned("a", 0, sha256_hex(b"")),
        ReturnManifestEntry::failed("a", ReturnPathsFailureCause::Missing),
    ] {
        assert_eq!(entry.chunks(), None);
        let wire = serde_json::to_value(&entry).unwrap();
        assert!(wire.get("chunks").is_none(), "{wire}");
        assert_eq!(
            serde_json::from_value::<ReturnManifestEntry>(wire).unwrap(),
            entry
        );
        for count in [0, 3, u32::MAX] {
            let counted = entry.clone().with_chunks(count);
            assert_eq!(counted.chunks(), Some(count));
            let wire = serde_json::to_value(&counted).unwrap();
            assert_eq!(wire["chunks"], count);
            assert_eq!(
                serde_json::from_value::<ReturnManifestEntry>(wire).unwrap(),
                counted
            );
        }
    }
    for count in [json!(-1), json!(4294967296u64), json!(1.5), json!("3")] {
        assert!(serde_json::from_value::<ReturnManifestEntry>(json!({
            "path": "a", "cause": "missing", "chunks": count
        }))
        .is_err());
    }
}

#[test]
fn counted_replay_preserves_original_counts_and_exact_seq_gap() {
    let retained = records("replay-retained-counted");
    let expired = records("replay-expired-counted");
    let get_manifest = |records: &[StreamRecord]| {
        records
            .iter()
            .find_map(|record| match record {
                StreamRecord::ReturnManifest(manifest) => Some(manifest.clone()),
                _ => None,
            })
            .unwrap()
    };
    let original = get_manifest(&retained);
    let replayed = get_manifest(&expired);
    assert_eq!(original.seq(), 6);
    assert_eq!(replayed.seq(), original.seq());
    let counts = |manifest: &ReturnManifest| {
        manifest
            .files()
            .iter()
            .map(|entry| entry.chunks())
            .collect::<Vec<_>>()
    };
    assert_eq!(counts(&original), [Some(3), Some(0)]);
    assert_eq!(counts(&replayed), counts(&original));
    let last_output_seq = expired
        .iter()
        .find_map(|record| match record {
            StreamRecord::Output(output) => Some(output.seq),
            _ => None,
        })
        .unwrap();
    assert_eq!(last_output_seq, 2);
    assert_eq!(
        replayed.seq() - last_output_seq - 1,
        replayed
            .files()
            .iter()
            .map(|entry| u64::from(entry.chunks().unwrap()))
            .sum::<u64>()
    );
    assert_eq!(
        retained
            .iter()
            .filter(|record| matches!(record, StreamRecord::ReturnChunk(_)))
            .count(),
        3
    );
    assert!(!expired
        .iter()
        .any(|record| matches!(record, StreamRecord::ReturnChunk(_))));
}

#[test]
fn constructors_produce_the_vector_records() {
    let raw = vector("two-files-one-empty")["records"].clone();
    let chunk = StreamRecord::ReturnChunk(ReturnChunk::new(
        job_id(),
        1,
        0,
        0,
        BytePayload(b"hello".to_vec()),
    ));
    let manifest = StreamRecord::ReturnManifest(ReturnManifest::new(
        job_id(),
        2,
        vec![
            ReturnManifestEntry::returned("dist/hello.txt", 5, sha256_hex(b"hello")),
            ReturnManifestEntry::returned("dist/empty.txt", 0, sha256_hex(b"")),
        ],
        5,
        RETURN_CHUNKED_MAX_BYTES_CAP,
    ));
    assert_eq!(serde_json::to_value(&chunk).unwrap(), raw[0]);
    assert_eq!(serde_json::to_value(&manifest).unwrap(), raw[1]);
    let StreamRecord::ReturnChunk(chunk) = chunk else {
        unreachable!()
    };
    assert_eq!(
        (
            chunk.job_id(),
            chunk.seq(),
            chunk.path_index(),
            chunk.offset(),
            chunk.data()
        ),
        (job_id(), 1, 0, 0, b"hello".as_slice())
    );
}

#[derive(Debug, PartialEq, Eq)]
enum OlderRecord {
    Known,
    Unknown { kind: String, seq: Option<u64> },
}

/// Decode a record with the tags a 0.2.9 reader knows, so a chunked record
/// lands in its unknown-record arm.
fn decode_as_older_reader(value: Value) -> OlderRecord {
    #[derive(Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    enum Known029 {
        Accepted,
        Started,
        Output,
        ReturnResult,
        Terminal,
    }
    #[derive(Deserialize)]
    struct Tag {
        #[serde(rename = "type")]
        kind: String,
        #[serde(default, deserialize_with = "seq")]
        seq: Option<u64>,
    }
    fn seq<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<u64>, D::Error> {
        u64::deserialize(deserializer).map(Some)
    }
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Input {
        Known(#[allow(dead_code)] Known029),
        Tag(Tag),
    }
    match serde_json::from_value::<Input>(value).unwrap() {
        Input::Known(_) => OlderRecord::Known,
        Input::Tag(Tag { kind, seq }) => OlderRecord::Unknown { kind, seq },
    }
}

#[test]
fn chunked_records_are_unknown_with_sequence_to_an_older_reader() {
    let raw = vector("three-chunks-short-last")["records"].clone();
    assert_eq!(
        decode_as_older_reader(raw[0].clone()),
        OlderRecord::Unknown {
            kind: "return_chunk".into(),
            seq: Some(1)
        }
    );
    assert_eq!(
        decode_as_older_reader(raw[3].clone()),
        OlderRecord::Unknown {
            kind: "return_manifest".into(),
            seq: Some(4)
        }
    );
    assert_eq!(decode_as_older_reader(raw[4].clone()), OlderRecord::Known);
}

#[test]
fn malformed_chunked_records_do_not_become_unknown() {
    for malformed in [
        json!({"type": "return_chunk", "seq": 1}),
        json!({"type": "return_chunk", "job_id": job_id(), "seq": 1, "path_index": 0,
               "offset": 0, "data": "not base64!"}),
        json!({"type": "return_manifest", "job_id": job_id(), "seq": 2, "files": []}),
    ] {
        assert!(
            serde_json::from_value::<StreamRecord>(malformed.clone()).is_err(),
            "{malformed}"
        );
    }
}

#[test]
fn chunked_records_ignore_unknown_fields() {
    let mut raw = vector("read-error-after-chunks")["records"].clone();
    for record in raw.as_array_mut().unwrap() {
        record["future_extension"] = json!(1);
        if let Some(files) = record.get_mut("files") {
            for entry in files.as_array_mut().unwrap() {
                entry["future_extension"] = json!(1);
            }
        }
    }
    let decoded: Vec<StreamRecord> = serde_json::from_value(raw).unwrap();
    assert_eq!(decoded, records("read-error-after-chunks"));
}

fn chunk(path_index: u32, offset: u64, data: &[u8]) -> ReturnChunk {
    ReturnChunk::new(job_id(), 1, path_index, offset, BytePayload(data.to_vec()))
}

#[test]
fn a_refused_chunk_leaves_the_assembler_unchanged() {
    let paths = vec!["a".to_owned(), "b".to_owned()];
    let mut assembler = ReturnAssembler::new(&paths, 6);
    assert_eq!(
        assembler.accept_chunk(&chunk(0, 0, b"aaa")).unwrap(),
        b"aaa"
    );
    let gap = assembler.accept_chunk(&chunk(0, 4, b"a")).unwrap_err();
    assert_eq!(
        gap,
        ReturnStreamError::NonContiguous {
            path_index: 0,
            offset: 4,
            expected_offset: 3
        }
    );
    assert!(matches!(
        assembler.accept_chunk(&chunk(1, 0, b"bbbb")),
        Err(ReturnStreamError::OverCap {
            path_index: 1,
            offset: 0,
            total_bytes: 7,
            cap_bytes: 6
        })
    ));
    assembler.accept_chunk(&chunk(1, 0, b"bbb")).unwrap();
    let verified = assembler
        .finish(&ReturnManifest::new(
            job_id(),
            3,
            vec![
                ReturnManifestEntry::returned("a", 3, sha256_hex(b"aaa")).with_chunks(1),
                ReturnManifestEntry::returned("b", 3, sha256_hex(b"bbb")).with_chunks(1),
            ],
            6,
            6,
        ))
        .unwrap();
    assert!(verified.iter().all(VerifiedReturn::is_returned));
}

#[test]
fn both_finishes_refuse_partial_chunk_counts() {
    let paths = vec!["a".into(), "b".into()];
    let manifest = ReturnManifest::new(
        job_id(),
        3,
        vec![
            ReturnManifestEntry::failed("a", ReturnPathsFailureCause::Missing).with_chunks(0),
            ReturnManifestEntry::failed("b", ReturnPathsFailureCause::Missing),
        ],
        0,
        10,
    );
    let assembler = ReturnAssembler::new(&paths, 10);
    for result in [
        assembler.clone().finish(&manifest),
        assembler.finish_expired_replay(&manifest, 2),
    ] {
        let error = result.unwrap_err();
        assert_eq!(
            error,
            ReturnStreamError::PartialChunkCounts { path_index: 1 }
        );
        assert_eq!(error.code(), "partial_chunk_counts");
        assert_eq!(error.path_index(), Some(1));
        assert_eq!(error.offset(), None);
        assert!(error.to_string().contains("entry 1"));
    }
}

#[test]
fn normal_finish_checks_read_error_chunk_counts() {
    let paths = vec!["a".into()];
    let mut assembler = ReturnAssembler::new(&paths, 10);
    assembler.accept_chunk(&chunk(0, 0, b"a")).unwrap();
    let manifest = ReturnManifest::new(
        job_id(),
        3,
        vec![ReturnManifestEntry::failed("a", ReturnPathsFailureCause::ReadError).with_chunks(2)],
        0,
        10,
    );
    let error = assembler.finish(&manifest).unwrap_err();
    assert_eq!(
        error,
        ReturnStreamError::ChunkCountMismatch {
            path_index: 0,
            chunks: 2,
            received: 1
        }
    );
    assert_eq!(error.code(), "chunk_count_mismatch");
    assert_eq!(error.path_index(), Some(0));
    assert_eq!(error.offset(), None);
    assert!(error
        .to_string()
        .contains("declares 2 chunks, but 1 were received"));
}

#[test]
fn expired_replay_refuses_a_wrong_seq_gap() {
    let paths = vec!["a".into()];
    for manifest_seq in [6, 2, 1, u64::MAX] {
        let manifest = ReturnManifest::new(
            job_id(),
            manifest_seq,
            vec![ReturnManifestEntry::failed("a", ReturnPathsFailureCause::Expired).with_chunks(2)],
            0,
            10,
        );
        let error = ReturnAssembler::new(&paths, 10)
            .finish_expired_replay(&manifest, 2)
            .unwrap_err();
        assert_eq!(
            error,
            ReturnStreamError::ReplayGapMismatch {
                last_output_seq: 2,
                manifest_seq,
                chunks: 2
            }
        );
        assert_eq!(error.code(), "replay_gap_mismatch");
        assert_eq!(error.path_index(), None);
        assert_eq!(error.offset(), None);
        assert!(error.to_string().contains("exactly 2 chunks"));
    }
}

#[test]
fn expired_replay_requires_no_chunks_and_preserves_failed_counts() {
    let paths = vec!["a".into()];
    let manifest = ReturnManifest::new(
        job_id(),
        5,
        vec![ReturnManifestEntry::failed("a", ReturnPathsFailureCause::ReadError).with_chunks(2)],
        0,
        10,
    );
    assert!(ReturnAssembler::new(&paths, 10)
        .finish_expired_replay(&manifest, 2)
        .is_ok());
    let mut assembler = ReturnAssembler::new(&paths, 10);
    assembler.accept_chunk(&chunk(0, 0, b"a")).unwrap();
    let error = assembler.finish_expired_replay(&manifest, 2).unwrap_err();
    assert_eq!(
        error,
        ReturnStreamError::ChunksInExpiredReplay {
            path_index: 0,
            received: 1
        }
    );
    assert_eq!(error.code(), "chunks_in_expired_replay");
    assert_eq!(error.path_index(), Some(0));
    assert_eq!(error.offset(), None);
    assert!(error.to_string().contains("expected none"));
    let legacy = ReturnManifest::new(
        job_id(),
        50,
        vec![ReturnManifestEntry::failed(
            "a",
            ReturnPathsFailureCause::Expired,
        )],
        0,
        10,
    );
    assert!(ReturnAssembler::new(&paths, 10)
        .finish_expired_replay(&legacy, 2)
        .is_ok());
}

#[test]
fn a_chunk_of_exactly_the_maximum_size_is_accepted() {
    let paths = vec!["big".to_owned()];
    let mut assembler = ReturnAssembler::new(&paths, RETURN_CHUNKED_MAX_BYTES_CAP);
    let data = vec![7u8; RETURN_CHUNK_MAX_BYTES];
    assert_eq!(
        assembler.accept_chunk(&chunk(0, 0, &data)).unwrap().len(),
        RETURN_CHUNK_MAX_BYTES
    );
    let tail = chunk(0, RETURN_CHUNK_MAX_BYTES as u64, b"x");
    assert_eq!(assembler.accept_chunk(&tail).unwrap(), b"x");
}

#[test]
fn finish_refuses_a_wrong_entry_count_and_total() {
    let paths = vec!["a".to_owned(), "b".to_owned()];
    let missing = ReturnManifestEntry::failed("a", ReturnPathsFailureCause::Missing);
    let short = ReturnAssembler::new(&paths, 10)
        .finish(&ReturnManifest::new(
            job_id(),
            1,
            vec![missing.clone()],
            0,
            10,
        ))
        .unwrap_err();
    assert_eq!(
        short,
        ReturnStreamError::EntryCountMismatch {
            path_index: 1,
            expected: 2,
            actual: 1
        }
    );
    assert_eq!(short.path_index(), Some(1));
    assert_eq!(short.offset(), None);
    let total = ReturnAssembler::new(&paths, 10)
        .finish(&ReturnManifest::new(
            job_id(),
            1,
            vec![
                missing,
                ReturnManifestEntry::returned("b", 0, sha256_hex(b"")),
            ],
            3,
            10,
        ))
        .unwrap_err();
    assert_eq!(
        total,
        ReturnStreamError::TotalMismatch {
            total_bytes: 3,
            returned_bytes: 0
        }
    );
    assert_eq!(total.path_index(), None);
}

#[test]
fn stream_errors_are_std_errors_naming_path_and_offset() {
    let error: Box<dyn std::error::Error> = Box::new(ReturnStreamError::NonContiguous {
        path_index: 2,
        offset: 6,
        expected_offset: 4,
    });
    assert_eq!(
        error.to_string(),
        "return_chunk for path_index 2 starts at offset 6, expected 4"
    );
    let failed = ReturnStreamError::ChunksForFailedPath {
        path_index: 0,
        cause: ReturnPathsFailureCause::OverCap,
        received: 4,
    };
    assert!(failed.to_string().contains("over_cap"), "{failed}");
    assert!(failed.to_string().contains("entry 0"), "{failed}");
}
