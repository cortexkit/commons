use cortexkit_exec_remote_types::*;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

const COPIED_OUTCOMES: &[&str] = &[
    "exit",
    "exit-nonzero",
    "signal",
    "cancelled",
    "outcome_unknown",
    "outcome_unknown-queued",
    "history_expired",
    "killed-deadline",
    "killed-cancel",
    "pipestatus",
    "unreachable",
    "runner_version_mismatch",
    "snapshot_failed",
    "workspace_key_rejected",
    "runner_full",
    "queue_wait_exceeded",
    "transfer_interrupted",
    "tree_hash_mismatch",
    "bundle_rejected",
    "workspace_setup_failed",
];
const LOCAL_OUTCOMES: &[&str] = &["crate-local-unknown-refusal", "crate-local-unknown-outcome"];
const COPIED_REPLIES: &[&str] = &[
    "prepare-prepared",
    "prepare-unreachable",
    "prepare-runner_version_mismatch",
    "prepare-snapshot_failed",
    "prepare-workspace_key_rejected",
    "prepare-transfer_interrupted",
    "prepare-bundle_rejected",
    "prepare-workspace_setup_failed",
    "drop-existing",
    "drop-missing",
    "status",
    "status-unreachable",
    "status-cold",
    "cancel",
];

#[derive(Debug, Serialize, Deserialize)]
struct OutcomeCase {
    request: RunRequest,
    stream: Vec<StreamRecord>,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/exec-remote-v1")
}

fn cases() -> Vec<(&'static str, &'static str)> {
    COPIED_OUTCOMES
        .iter()
        .chain(LOCAL_OUTCOMES)
        .map(|name| ("outcomes", *name))
        .chain(COPIED_REPLIES.iter().map(|name| ("replies", *name)))
        .collect()
}

fn bytes(folder: &str, name: &str) -> Vec<u8> {
    fs::read(root().join(folder).join(format!("{name}.jcs"))).unwrap()
}

fn value(folder: &str, name: &str) -> Value {
    serde_json::from_slice(&bytes(folder, name)).unwrap()
}

fn round_trip<T: DeserializeOwned + Serialize>(bytes: &[u8]) -> Vec<u8> {
    let decoded: T = serde_json::from_slice(bytes).unwrap();
    serde_json_canonicalizer::to_vec(&decoded).unwrap()
}

fn canonical_reply(name: &str, bytes: &[u8]) -> Vec<u8> {
    if name.starts_with("prepare-") {
        round_trip::<PrepareReply>(bytes)
    } else if name.starts_with("drop-") {
        round_trip::<DropReply>(bytes)
    } else if name.starts_with("status") {
        round_trip::<StatusReply>(bytes)
    } else {
        assert_eq!(name, "cancel", "every reply must have a declared type");
        round_trip::<CancelReply>(bytes)
    }
}

#[test]
fn golden_vector_inventory_is_complete() {
    for (folder, names) in [
        (
            "outcomes",
            COPIED_OUTCOMES
                .iter()
                .chain(LOCAL_OUTCOMES)
                .copied()
                .collect::<Vec<_>>(),
        ),
        ("replies", COPIED_REPLIES.to_vec()),
    ] {
        let expected: BTreeSet<_> = names
            .iter()
            .flat_map(|name| [format!("{name}.jcs"), format!("{name}.sha256")])
            .collect();
        let actual: BTreeSet<_> = fs::read_dir(root().join(folder))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(actual, expected, "missing or unchecked {folder} cases");
    }
    assert_eq!(COPIED_OUTCOMES.len(), 20);
    assert_eq!(COPIED_REPLIES.len(), 14);
    assert_eq!(cases().len(), 36);
}

#[test]
fn golden_vector_sha256_matches_jcs_bytes() {
    for (folder, name) in cases() {
        // Hash the pinned file, not a re-encoding that could mask changed bytes.
        let hash = format!("{:x}", Sha256::digest(bytes(folder, name)));
        let expected =
            fs::read_to_string(root().join(folder).join(format!("{name}.sha256"))).unwrap();
        assert_eq!(expected, hash, "{folder}/{name} SHA-256");
    }
}

#[test]
fn golden_vectors_round_trip_to_pinned_canonical_bytes() {
    for (folder, name) in cases() {
        let pinned = bytes(folder, name);
        let canonical = if folder == "outcomes" {
            round_trip::<OutcomeCase>(&pinned)
        } else {
            canonical_reply(name, &pinned)
        };
        assert_eq!(
            canonical, pinned,
            "{folder}/{name} typed RFC 8785 round-trip"
        );
    }
}

fn add_unknown_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, child) in object.iter_mut() {
                // env is an open caller-supplied map, not a typed record.
                if key != "env" {
                    add_unknown_fields(child);
                }
            }
            object.insert("future_extension".into(), json!({"version": 2}));
        }
        Value::Array(array) => array.iter_mut().for_each(add_unknown_fields),
        _ => {}
    }
}

fn ignores_unknown<T: DeserializeOwned + Serialize>(request: T) {
    let original = serde_json::to_value(request).unwrap();
    let mut extended = original.clone();
    add_unknown_fields(&mut extended);
    let canonical = round_trip::<T>(&serde_json::to_vec(&extended).unwrap());
    assert_eq!(
        canonical,
        serde_json_canonicalizer::to_vec(&original).unwrap()
    );
}

#[test]
fn all_caller_records_ignore_unknown_fields() {
    for (folder, name) in cases() {
        let pinned = bytes(folder, name);
        let mut extended: Value = serde_json::from_slice(&pinned).unwrap();
        add_unknown_fields(&mut extended);
        let extended = serde_json::to_vec(&extended).unwrap();
        let canonical = if folder == "outcomes" {
            round_trip::<OutcomeCase>(&extended)
        } else {
            canonical_reply(name, &extended)
        };
        assert_eq!(canonical, pinned, "{folder}/{name} ignores additive fields");
    }
    ignores_unknown(AttachRequest::new(job_id(), 7));
    ignores_unknown(CancelRequest::new(job_id()));
    ignores_unknown(DropRequest::new("/workspace"));
    ignores_unknown(PrepareRequest::new("/workspace", "/repo", "base"));
    ignores_unknown(StatusRequest::new());
}

fn job_id() -> Uuid {
    "0192a64a-1234-7000-8000-000000000001".parse().unwrap()
}

#[test]
fn split_utf8_output_chunks_round_trip_exact_bytes_and_sequence() {
    // U+20AC is split between records, so neither chunk is valid UTF-8 alone.
    for (seq, raw, encoded) in [(7, vec![0xe2], "4g=="), (8, vec![0x82, 0xac], "gqw=")] {
        assert!(std::str::from_utf8(&raw).is_err());
        let record = StreamRecord::Output(Output::new(
            seq,
            OutputStream::Stdout,
            BytePayload(raw.clone()),
        ));
        let expected = json!({"type": "output", "seq": seq, "stream": "stdout", "bytes": encoded});
        assert_eq!(serde_json::to_value(&record).unwrap(), expected);
        let decoded: StreamRecord = serde_json::from_value(expected).unwrap();
        let StreamRecord::Output(output) = decoded else {
            panic!("expected output")
        };
        assert_eq!(output.seq, seq);
        assert_eq!(output.bytes.0, raw);
    }
}

#[test]
fn invalid_base64_and_uuid_syntax_are_rejected() {
    for encoded in ["not base64!", "4g", "_w=="] {
        assert!(
            serde_json::from_value::<BytePayload>(json!(encoded)).is_err(),
            "{encoded}"
        );
    }
    for bad in ["not-a-uuid", "", "0192a64a-1234-7000-8000-00000000000z"] {
        assert!(serde_json::from_value::<CancelRequest>(json!({"job_id": bad})).is_err());
        assert!(serde_json::from_value::<PrepareReply>(
            json!({"transfer_id": bad, "outcome": {"type": "prepared"}})
        )
        .is_err());
    }
}

#[test]
fn unknown_outcome_tag_is_retained_and_round_trips() {
    let raw = value("outcomes", "crate-local-unknown-outcome")["stream"][0]["outcome"].clone();
    let outcome: Outcome = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(
        outcome,
        Outcome::Unknown {
            kind: "future_outcome".into()
        }
    );
    assert_eq!(serde_json::to_value(outcome).unwrap(), raw);
    let extended: Outcome = serde_json::from_value(
        json!({"type": "future_outcome", "future_field": {"code": "opaque"}}),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_value(extended).unwrap(),
        json!({"type": "future_outcome"})
    );
}

#[test]
fn unknown_refusal_tag_retains_before_start_arm_and_round_trips() {
    let raw = value("outcomes", "crate-local-unknown-refusal")["stream"][0]["outcome"].clone();
    let outcome: Outcome = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(
        outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::Unknown("future_refusal".into())
        }
    );
    assert_eq!(serde_json::to_value(outcome).unwrap(), raw);
    let prepare: PrepareOutcome = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(
        prepare,
        PrepareOutcome::RefusedBeforeStart {
            reason: RefusalReason::Unknown("future_refusal".into())
        }
    );
    assert_eq!(serde_json::to_value(prepare).unwrap(), raw);
}

#[test]
fn known_outcome_tags_with_malformed_fields_do_not_become_unknown() {
    for malformed in [
        json!({"type": "exit"}),
        json!({"type": "exit", "code": "0"}),
        json!({"type": "exit", "code": 2147483648_i64}),
        json!({"type": "signal"}),
        json!({"type": "signal", "signal": null}),
        json!({"type": "refused_before_start"}),
        json!({"type": "refused_before_start", "reason": 1}),
        json!({"type": "refused_before_start", "reason": {"type": "future_refusal"}}),
        json!({"type": 1}),
        json!({"code": 0}),
        json!([]),
    ] {
        assert!(
            serde_json::from_value::<Outcome>(malformed.clone()).is_err(),
            "{malformed}"
        );
    }
    assert!(serde_json::from_str::<Outcome>(r#"{"type":"exit","code":0,"code":1}"#).is_err());
    assert!(
        serde_json::from_str::<Outcome>(r#"{"type":"exit","type":"future_outcome","code":0}"#)
            .is_err()
    );
    assert!(
        serde_json::from_value::<PrepareOutcome>(json!({"type": "refused_before_start"})).is_err()
    );
}

#[test]
fn known_outcome_tags_keep_their_semantic_variants() {
    for (wire, expected) in [
        (
            json!({"type": "exit", "code": 7}),
            Outcome::Exit { code: 7 },
        ),
        (
            json!({"type": "signal", "signal": 15}),
            Outcome::Signal { signal: 15 },
        ),
        (json!({"type": "cancelled"}), Outcome::Cancelled),
        (json!({"type": "outcome_unknown"}), Outcome::OutcomeUnknown),
        (json!({"type": "history_expired"}), Outcome::HistoryExpired),
    ] {
        assert_eq!(serde_json::from_value::<Outcome>(wire).unwrap(), expected);
    }
    for (tag, reason) in [
        ("unreachable", RefusalReason::Unreachable),
        (
            "runner_version_mismatch",
            RefusalReason::RunnerVersionMismatch,
        ),
        ("snapshot_failed", RefusalReason::SnapshotFailed),
        (
            "workspace_key_rejected",
            RefusalReason::WorkspaceKeyRejected,
        ),
        ("runner_full", RefusalReason::RunnerFull),
        ("queue_wait_exceeded", RefusalReason::QueueWaitExceeded),
        ("transfer_interrupted", RefusalReason::TransferInterrupted),
        ("tree_hash_mismatch", RefusalReason::TreeHashMismatch),
        ("bundle_rejected", RefusalReason::BundleRejected),
        (
            "workspace_setup_failed",
            RefusalReason::WorkspaceSetupFailed,
        ),
    ] {
        assert_eq!(
            serde_json::from_value::<Outcome>(
                json!({"type": "refused_before_start", "reason": tag})
            )
            .unwrap(),
            Outcome::RefusedBeforeStart { reason }
        );
    }
}

#[test]
fn other_enum_tags_remain_strict() {
    assert!(serde_json::from_value::<StreamRecord>(json!({"type": "future_record"})).is_err());
    assert!(serde_json::from_value::<PrepareOutcome>(json!({"type": "future_prepare"})).is_err());
    assert!(serde_json::from_value::<Killed>(json!("future_kill")).is_err());
    assert!(serde_json::from_value::<Ran>(json!("local")).is_err());
    assert!(serde_json::from_value::<OutputStream>(json!("future_stream")).is_err());
    assert!(serde_json::from_value::<RebuildResult>(json!("future_result")).is_err());
}

#[test]
fn availability_fields_are_explicit_nulls_not_empty_lists() {
    let absent = TerminalRecord::new(job_id(), Outcome::HistoryExpired, 0, 0, 0);
    let encoded = serde_json::to_value(&absent).unwrap();
    for field in ["ran", "tree_hash", "workspace_changes"] {
        assert_eq!(
            encoded.get(field),
            Some(&Value::Null),
            "{field} must be emitted"
        );
    }
    assert!(encoded.get("killed").is_none());
    assert!(encoded.get("pipestatus").is_none());
    let remote = absent
        .with_ran(Ran::Remote)
        .with_workspace_changes(Vec::new());
    let encoded = serde_json::to_value(remote).unwrap();
    assert_eq!(encoded["ran"], "remote");
    assert_eq!(encoded["workspace_changes"], json!([]));
    let before_start =
        TerminalRecord::new(job_id(), Outcome::Cancelled, 0, 0, 0).with_ran(Ran::None);
    assert_eq!(serde_json::to_value(before_start).unwrap()["ran"], "none");

    let cold = serde_json::to_value(RepositoryStatus::new("/repo")).unwrap();
    for field in [
        "warm_target_age_s",
        "published_commit",
        "last_rebuild_result",
    ] {
        assert_eq!(
            cold.get(field),
            Some(&Value::Null),
            "{field} must be emitted"
        );
    }
}

#[test]
fn constructors_and_setters_preserve_caller_wire_shapes() {
    let request = RunRequest::new("/workspace", "/repo", "/workspace", "true");
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "workspace_key": "/workspace", "repository_root": "/repo", "cwd": "/workspace",
            "command": "true", "env": {}, "siblings": []
        })
    );
    let minimal: RunRequest = serde_json::from_value(json!({
        "workspace_key": "/workspace", "repository_root": "/repo", "cwd": "/workspace", "command": "true"
    })).unwrap();
    assert_eq!(minimal, request);
    let request = request
        .with_env([("RUST_BACKTRACE".into(), "1".into())].into())
        .with_weight_hint(16)
        .with_timeout(60)
        .with_queue_wait_limit_s(0)
        .with_siblings(vec!["/sibling".into()]);
    assert_eq!(
        serde_json::to_value(request).unwrap(),
        json!({
            "workspace_key": "/workspace", "repository_root": "/repo", "cwd": "/workspace", "command": "true",
            "env": {"RUST_BACKTRACE": "1"}, "weight_hint": 16, "timeout": 60, "queue_wait_limit_s": 0, "siblings": ["/sibling"]
        })
    );
    let terminal = TerminalRecord::new(job_id(), Outcome::Exit { code: 0 }, 125, 25, 29)
        .with_killed(Killed::Deadline)
        .with_pipestatus(vec![0, 1])
        .with_ran(Ran::Remote)
        .with_tree_hash("hash")
        .with_workspace_changes(vec!["result.txt".into()]);
    assert_eq!(
        serde_json::to_value(terminal).unwrap(),
        json!({
            "job_id": job_id(), "outcome": {"type": "exit", "code": 0}, "killed": "deadline", "pipestatus": [0, 1],
            "wall_ms": 125, "queue_wait_ms": 25, "bundle_bytes": 29, "ran": "remote", "tree_hash": "hash", "workspace_changes": ["result.txt"]
        })
    );
    let output =
        Output::new(8, OutputStream::Stderr, BytePayload(vec![0xff])).with_truncated_before_seq(7);
    assert_eq!(
        serde_json::to_value(output).unwrap(),
        json!({"seq": 8, "stream": "stderr", "bytes": "/w==", "truncated_before_seq": 7})
    );
    let repository = RepositoryStatus::new("/repo")
        .with_warm_target_age_s(3)
        .with_published_commit("commit")
        .with_last_rebuild_result(RebuildResult::Ok);
    assert_eq!(
        serde_json::to_value(repository).unwrap(),
        json!({
            "repository_root": "/repo", "warm_target_age_s": 3, "published_commit": "commit", "last_rebuild_result": "ok"
        })
    );
    let prepare =
        PrepareRequest::new("/workspace", "/repo", "base").with_siblings(vec!["/sibling".into()]);
    assert_eq!(
        serde_json::to_value(prepare).unwrap(),
        json!({"workspace_key": "/workspace", "repository_root": "/repo", "base_commit": "base", "siblings": ["/sibling"]})
    );
    assert_eq!(
        serde_json::to_value(StatusRequest::new()).unwrap(),
        json!({})
    );
    assert_eq!(
        serde_json::to_value(AttachRequest::new(job_id(), 7)).unwrap(),
        json!({"job_id": job_id(), "from_seq": 7})
    );
    assert_eq!(
        serde_json::to_value(Accepted::new(job_id(), 1)).unwrap(),
        json!({"job_id": job_id(), "queue_position": 1})
    );
    assert_eq!(
        serde_json::to_value(CancelReply::new(job_id())).unwrap(),
        json!({"job_id": job_id()})
    );
    assert_eq!(
        serde_json::to_value(DropReply::new(true, vec![job_id()])).unwrap(),
        json!({"dropped": true, "cancelled_jobs": [job_id()]})
    );
    assert_eq!(
        serde_json::to_value(PrepareReply::new(job_id(), PrepareOutcome::Prepared)).unwrap(),
        json!({"transfer_id": job_id(), "outcome": {"type": "prepared"}})
    );
    let status = StatusReply::new(
        1,
        vec![RunningJob::new(job_id(), "base:/repo", 16)],
        true,
        vec![],
        "rustc 1.99.0",
    );
    assert_eq!(
        serde_json::to_value(status).unwrap(),
        json!({"queue_depth": 1, "running_jobs": [{"job_id": job_id(), "workspace_key": "base:/repo", "weight": 16}], "server_reachable": true, "repositories": [], "rustc_version": "rustc 1.99.0"})
    );
}
