use cortexkit_exec_remote_types::*;
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
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
const LOCAL_OUTCOMES: &[&str] = &[
    "crate-local-unknown-refusal",
    "crate-local-unknown-outcome",
    "crate-local-unknown-stream-record",
    "crate-local-unknown-killed",
    "crate-local-unknown-ran",
    "crate-local-unknown-output-stream",
    "crate-local-server-reports-all",
    "crate-local-server-reports-older-runner",
    "crate-local-server-reports-unknown-fields",
    "crate-local-server-reports-detached-head",
    "crate-local-server-reports-truncated-untracked",
    "crate-local-server-reports-unchanged",
    "crate-local-refusal-hints",
    "crate-local-runner-draining",
    "crate-local-runner-disk-full",
    "crate-local-started",
    "crate-local-network-outbound",
    "crate-local-unknown-network",
    "crate-local-network-unsupported",
    "crate-local-platform-linux",
    "crate-local-platform-windows",
    "crate-local-platform-unsupported",
    "crate-local-unknown-platform",
];
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
const LOCAL_REPLIES: &[&str] = &[
    "crate-local-unknown-prepare-outcome",
    "crate-local-unknown-rebuild-result",
    "crate-local-prepare-refusal-hints",
];

#[derive(Debug, Serialize, Deserialize)]
struct OutcomeCase {
    request: RunRequest,
    stream: Vec<StreamRecord>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum KnownStreamRecord023 {
    Accepted,
    Output,
    Terminal,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StreamRecord023Input {
    Known(KnownStreamRecord023),
    Tag(StreamRecord023Tag),
}

#[derive(Debug, Deserialize)]
struct StreamRecord023Tag {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "deserialize_seq_023")]
    seq: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
enum StreamRecord023 {
    Accepted,
    Output,
    Terminal,
    Unknown { kind: String, seq: Option<u64> },
}

fn deserialize_seq_023<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<u64>, D::Error> {
    u64::deserialize(deserializer).map(Some)
}

fn decode_stream_record_023(value: Value) -> StreamRecord023 {
    match serde_json::from_value::<StreamRecord023Input>(value).unwrap() {
        StreamRecord023Input::Known(KnownStreamRecord023::Accepted) => StreamRecord023::Accepted,
        StreamRecord023Input::Known(KnownStreamRecord023::Output) => StreamRecord023::Output,
        StreamRecord023Input::Known(KnownStreamRecord023::Terminal) => StreamRecord023::Terminal,
        StreamRecord023Input::Tag(StreamRecord023Tag { kind, seq }) => {
            StreamRecord023::Unknown { kind, seq }
        }
    }
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/exec-remote-v1")
}

fn cases() -> Vec<(&'static str, &'static str)> {
    COPIED_OUTCOMES
        .iter()
        .chain(LOCAL_OUTCOMES)
        .map(|name| ("outcomes", *name))
        .chain(
            COPIED_REPLIES
                .iter()
                .chain(LOCAL_REPLIES)
                .map(|name| ("replies", *name)),
        )
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

fn expected_canonical_bytes(folder: &str, name: &str) -> Vec<u8> {
    // Unknown object fields are ignored, so this input re-encodes to the
    // separately pinned report containing only known fields.
    if folder == "outcomes" && name == "crate-local-server-reports-unknown-fields" {
        bytes(folder, "crate-local-server-reports-all")
    } else {
        bytes(folder, name)
    }
}

fn canonical_reply(name: &str, bytes: &[u8]) -> Vec<u8> {
    if name.starts_with("prepare-")
        || name == "crate-local-unknown-prepare-outcome"
        || name == "crate-local-prepare-refusal-hints"
    {
        round_trip::<PrepareReply>(bytes)
    } else if name.starts_with("drop-") {
        round_trip::<DropReply>(bytes)
    } else if name.starts_with("status") || name == "crate-local-unknown-rebuild-result" {
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
        (
            "replies",
            COPIED_REPLIES
                .iter()
                .chain(LOCAL_REPLIES)
                .copied()
                .collect::<Vec<_>>(),
        ),
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
    assert_eq!(LOCAL_OUTCOMES.len(), 23);
    assert_eq!(LOCAL_REPLIES.len(), 3);
    assert_eq!(cases().len(), 60);
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
            canonical,
            expected_canonical_bytes(folder, name),
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
        assert_eq!(
            canonical,
            expected_canonical_bytes(folder, name),
            "{folder}/{name} ignores additive fields"
        );
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

fn server_report(name: &str) -> TerminalRecord {
    let case: OutcomeCase = serde_json::from_slice(&bytes("outcomes", name)).unwrap();
    let [StreamRecord::Terminal(terminal)] = case.stream.as_slice() else {
        panic!("expected one terminal in {name}")
    };
    terminal.clone()
}

#[test]
fn server_reports_all_fields_match_vector_and_round_trip() {
    let state = GitStateChange::new(0, 1)
        .with_head_before("1111111111111111111111111111111111111111")
        .with_head_after("2222222222222222222222222222222222222222")
        .with_ref_before("refs/heads/main")
        .with_ref_after("refs/heads/build")
        .with_index_tree_before("4444444444444444444444444444444444444444")
        .with_index_tree_after("5555555555555555555555555555555555555555");
    assert!(state.changed());
    let report = TerminalRecord::new(job_id(), Outcome::Exit { code: 0 }, 125, 25, 29)
        .with_ran(Ran::Remote)
        .with_tree_hash("3333333333333333333333333333333333333333")
        .with_workspace_changes(vec!["result.txt".into()])
        .with_git_state_changed(state)
        .with_untracked_files(UntrackedFiles::new(vec![
            "generated/new.txt".into(),
            "notes.txt".into(),
        ]))
        .with_ignored_writes(
            IgnoredWrites::new(25)
                .with_sample_paths(vec!["scratch/debug.log".into(), "cache/result.bin".into()]),
        );
    assert_eq!(report, server_report("crate-local-server-reports-all"));
    assert_eq!(
        serde_json::to_value(StreamRecord::Terminal(report)).unwrap(),
        value("outcomes", "crate-local-server-reports-all")["stream"][0]
    );
}

#[test]
fn server_reports_older_runner_decodes_as_not_reported() {
    let report = server_report("crate-local-server-reports-older-runner");
    assert_eq!(report.git_state_changed, None);
    assert_eq!(report.untracked_files, None);
    assert_eq!(report.ignored_writes, None);
    let new = TerminalRecord::new(job_id(), Outcome::Exit { code: 0 }, 0, 0, 0);
    assert_eq!(new.git_state_changed, None);
    assert_eq!(new.untracked_files, None);
    assert_eq!(new.ignored_writes, None);
    let encoded = serde_json::to_value(StreamRecord::Terminal(report)).unwrap();
    for field in ["git_state_changed", "untracked_files", "ignored_writes"] {
        assert!(encoded.get(field).is_none(), "{field} must be omitted");
    }
    assert_eq!(
        encoded,
        value("outcomes", "crate-local-server-reports-older-runner")["stream"][0]
    );
}

#[test]
fn server_reports_unknown_fields_are_tolerated() {
    let raw = value("outcomes", "crate-local-server-reports-unknown-fields");
    for field in ["git_state_changed", "untracked_files", "ignored_writes"] {
        assert_eq!(
            raw["stream"][0][field]["future_extension"],
            json!({"version": 3})
        );
    }
    let report = server_report("crate-local-server-reports-unknown-fields");
    assert_eq!(report, server_report("crate-local-server-reports-all"));
    assert_eq!(
        serde_json::to_value(StreamRecord::Terminal(report)).unwrap(),
        value("outcomes", "crate-local-server-reports-all")["stream"][0]
    );
}

#[test]
fn server_reports_detached_head_keeps_null_refs_and_unavailable_index_tree() {
    let report = server_report("crate-local-server-reports-detached-head");
    let state = report.git_state_changed.as_ref().unwrap();
    assert_eq!(state.ref_before, None);
    assert_eq!(state.ref_after, None);
    assert_eq!(state.index_tree_before, None);
    assert_eq!(
        state.index_tree_after.as_deref(),
        Some("5555555555555555555555555555555555555555")
    );
    assert_eq!(
        state.head_before.as_deref(),
        Some("1111111111111111111111111111111111111111")
    );
    assert_eq!(
        state.head_after.as_deref(),
        Some("2222222222222222222222222222222222222222")
    );
    assert!(state.changed());
    assert_eq!(
        serde_json::to_value(StreamRecord::Terminal(report)).unwrap(),
        value("outcomes", "crate-local-server-reports-detached-head")["stream"][0]
    );
}

#[test]
fn server_reports_truncated_untracked_list_is_explicit() {
    let report = server_report("crate-local-server-reports-truncated-untracked");
    assert_eq!(
        report.untracked_files,
        Some(
            UntrackedFiles::new(vec!["generated/new.txt".into(), "notes.txt".into()])
                .with_truncated(true)
        )
    );
    assert_eq!(
        serde_json::to_value(StreamRecord::Terminal(report)).unwrap(),
        value("outcomes", "crate-local-server-reports-truncated-untracked")["stream"][0]
    );
}

#[test]
fn server_reports_unchanged_is_present_not_unreported() {
    let report = server_report("crate-local-server-reports-unchanged");
    assert!(!report.git_state_changed.as_ref().unwrap().changed());
    assert_eq!(
        report.untracked_files,
        Some(UntrackedFiles::new(Vec::new()))
    );
    assert_eq!(report.ignored_writes, Some(IgnoredWrites::new(0)));
    assert_eq!(
        serde_json::to_value(StreamRecord::Terminal(report)).unwrap(),
        value("outcomes", "crate-local-server-reports-unchanged")["stream"][0]
    );
}

#[test]
fn git_state_changed_checks_each_pair_and_availability() {
    let unchanged = GitStateChange::new(0, 0);
    assert!(!unchanged.changed());
    // Exercise each comparison independently, rather than only a report in
    // which every pair changes and an omitted comparison could go unnoticed.
    for changed in [
        unchanged.clone().with_head_before("commit"),
        unchanged.clone().with_head_after("commit"),
        unchanged.clone().with_ref_before("refs/heads/main"),
        unchanged.clone().with_ref_after("refs/heads/main"),
        unchanged.clone().with_index_tree_before("tree"),
        unchanged.clone().with_index_tree_after("tree"),
        GitStateChange::new(0, 1),
        GitStateChange::new(1, 0),
    ] {
        assert!(changed.changed(), "{changed:?}");
    }
    assert_eq!(
        serde_json::to_value(unchanged).unwrap(),
        json!({"head_before": null, "head_after": null, "ref_before": null,
            "ref_after": null, "index_tree_before": null, "index_tree_after": null,
            "stash_count_before": 0, "stash_count_after": 0})
    );
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
fn unknown_stream_record_retains_sequence_and_round_trips() {
    let raw = value("outcomes", "crate-local-unknown-stream-record")["stream"][0].clone();
    let record: StreamRecord = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(
        record,
        StreamRecord::Unknown {
            kind: "future_record".into(),
            seq: Some(7),
        }
    );
    assert_eq!(serde_json::to_value(record).unwrap(), raw);

    for seq in [None, Some(0), Some(u64::MAX)] {
        let mut raw = json!({"type": "future_record"});
        if let Some(seq) = seq {
            raw["seq"] = json!(seq);
        }
        let mut extended = raw.clone();
        extended["future_field"] = json!({"opaque": true});
        let decoded: StreamRecord = serde_json::from_value(extended).unwrap();
        assert_eq!(
            decoded,
            StreamRecord::Unknown {
                kind: "future_record".into(),
                seq,
            }
        );
        assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
    }

    let case: OutcomeCase =
        serde_json::from_value(value("outcomes", "crate-local-unknown-stream-record")).unwrap();
    assert!(matches!(case.stream[1], StreamRecord::Terminal(_)));
}

#[test]
fn started_record_round_trips_its_golden_vector() {
    let raw = value("outcomes", "crate-local-started")["stream"][1].clone();
    let record: StreamRecord = serde_json::from_value(raw.clone()).unwrap();
    let started = Started::new(7, 125, 1_730_000_000_125);
    assert_eq!(record, StreamRecord::Started(started.clone()));
    assert_eq!(started.seq(), 7);
    assert_eq!(started.queue_wait_ms(), 125);
    assert_eq!(started.started_at_ms(), 1_730_000_000_125);
    assert_eq!(serde_json::to_value(record).unwrap(), raw);
}

#[test]
fn started_record_is_unknown_with_sequence_to_a_023_decoder() {
    let raw = value("outcomes", "crate-local-started")["stream"][1].clone();
    assert_eq!(
        decode_stream_record_023(raw),
        StreamRecord023::Unknown {
            kind: "started".into(),
            seq: Some(7),
        }
    );
}

#[test]
fn started_sequence_advances_resume_cursor_and_is_nonterminal() {
    let case: OutcomeCase =
        serde_json::from_value(value("outcomes", "crate-local-started")).unwrap();
    let [StreamRecord::Accepted(_), StreamRecord::Started(started), StreamRecord::Terminal(_)] =
        case.stream.as_slice()
    else {
        panic!("expected acceptance, start, and terminal records in order")
    };
    let attach = AttachRequest::new(job_id(), started.seq() + 1);
    assert_eq!(attach.from_seq, 8);
}

#[test]
fn malformed_started_stream_record_is_rejected() {
    for malformed in [
        json!({"type": "started"}),
        json!({"type": "started", "seq": "7", "queue_wait_ms": 125, "started_at_ms": 1_730_000_000_125_u64}),
        json!({"type": "started", "seq": 7, "queue_wait_ms": 125}),
        json!({"type": "started", "seq": 7, "queue_wait_ms": null, "started_at_ms": 1_730_000_000_125_u64}),
    ] {
        assert!(
            serde_json::from_value::<StreamRecord>(malformed.clone()).is_err(),
            "{malformed}"
        );
    }
}

#[test]
fn unknown_stream_record_with_non_u64_sequence_is_rejected() {
    for seq in [
        json!(null),
        json!(-1),
        json!(1.5),
        json!("7"),
        json!(true),
        json!([]),
        json!({}),
    ] {
        assert!(
            serde_json::from_value::<StreamRecord>(json!({"type": "future_record", "seq": seq}))
                .is_err(),
            "{seq}"
        );
    }
    assert!(serde_json::from_str::<StreamRecord>(
        r#"{"type":"future_record","seq":18446744073709551616}"#
    )
    .is_err());
}

#[test]
fn unknown_killed_tag_is_retained_and_round_trips() {
    let raw = value("outcomes", "crate-local-unknown-killed")["stream"][0].clone();
    let record: StreamRecord = serde_json::from_value(raw.clone()).unwrap();
    let StreamRecord::Terminal(terminal) = &record else {
        panic!("expected terminal")
    };
    assert_eq!(terminal.killed, Some(Killed::Unknown("future_kill".into())));
    assert_eq!(serde_json::to_value(record).unwrap(), raw);
}

#[test]
fn unknown_ran_tag_is_not_none_and_round_trips() {
    let raw = value("outcomes", "crate-local-unknown-ran")["stream"][0].clone();
    let record: StreamRecord = serde_json::from_value(raw.clone()).unwrap();
    let StreamRecord::Terminal(terminal) = &record else {
        panic!("expected terminal")
    };
    assert_eq!(terminal.ran, Some(Ran::Unknown("future_location".into())));
    assert_ne!(terminal.ran, Some(Ran::None));
    assert_eq!(serde_json::to_value(record).unwrap(), raw);
}

#[test]
fn unknown_output_stream_delivers_bytes_and_sequence_and_round_trips() {
    let raw = value("outcomes", "crate-local-unknown-output-stream")["stream"][0].clone();
    let record: StreamRecord = serde_json::from_value(raw.clone()).unwrap();
    let StreamRecord::Output(output) = &record else {
        panic!("expected output")
    };
    assert_eq!(output.stream, OutputStream::Unknown("future_stream".into()));
    assert_eq!(output.seq, 7);
    assert_eq!(output.bytes.0, vec![0xe2]);
    assert_eq!(serde_json::to_value(record).unwrap(), raw);
}

#[test]
fn unknown_prepare_outcome_is_not_prepared_and_round_trips() {
    let raw = value("replies", "crate-local-unknown-prepare-outcome");
    let reply: PrepareReply = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(
        reply.outcome,
        PrepareOutcome::Unknown {
            kind: "future_prepare".into()
        }
    );
    assert_ne!(reply.outcome, PrepareOutcome::Prepared);
    assert_eq!(serde_json::to_value(reply).unwrap(), raw);
    let extended: PrepareOutcome =
        serde_json::from_value(json!({"type": "future_prepare", "future_field": {"opaque": true}}))
            .unwrap();
    assert_eq!(
        serde_json::to_value(extended).unwrap(),
        json!({"type": "future_prepare"})
    );
}

#[test]
fn unknown_rebuild_result_is_retained_and_round_trips() {
    let raw = value("replies", "crate-local-unknown-rebuild-result");
    let reply: StatusReply = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(
        reply.repositories[0].last_rebuild_result,
        Some(RebuildResult::Unknown("future_result".into()))
    );
    assert_eq!(serde_json::to_value(reply).unwrap(), raw);
}

#[test]
fn known_stream_record_tags_with_malformed_fields_do_not_become_unknown() {
    for malformed in [
        json!({"type": "accepted"}),
        json!({"type": "accepted", "job_id": "not-a-uuid", "queue_position": 1}),
        json!({"type": "accepted", "job_id": job_id(), "queue_position": -1}),
        json!({"type": "output"}),
        json!({"type": "output", "seq": 7, "stream": "stdout"}),
        json!({"type": "output", "seq": "7", "stream": "stdout", "bytes": "4g=="}),
        json!({"type": "output", "seq": 7, "stream": 1, "bytes": "4g=="}),
        json!({"type": "output", "seq": 7, "stream": "stdout", "bytes": "not base64!"}),
        json!({"type": "terminal"}),
        json!({"type": 1}),
        json!({"seq": 7}),
        json!([]),
    ] {
        assert!(
            serde_json::from_value::<StreamRecord>(malformed.clone()).is_err(),
            "{malformed}"
        );
    }
    let terminal = value("outcomes", "crate-local-unknown-outcome")["stream"][0].clone();
    for field in [
        "job_id",
        "outcome",
        "wall_ms",
        "queue_wait_ms",
        "bundle_bytes",
    ] {
        let mut malformed = terminal.clone();
        malformed.as_object_mut().unwrap().remove(field);
        assert!(
            serde_json::from_value::<StreamRecord>(malformed).is_err(),
            "missing {field}"
        );
    }
    assert!(serde_json::from_str::<StreamRecord>(
        r#"{"type":"output","seq":7,"seq":8,"stream":"stdout","bytes":"4g=="}"#
    )
    .is_err());
    assert!(serde_json::from_str::<StreamRecord>(
        r#"{"type":"output","type":"future_record","seq":7,"stream":"stdout","bytes":"4g=="}"#
    )
    .is_err());
}

#[test]
fn known_prepare_outcome_tags_with_malformed_fields_do_not_become_unknown() {
    for malformed in [
        json!({"type": "refused_before_start"}),
        json!({"type": "refused_before_start", "reason": null}),
        json!({"type": "refused_before_start", "reason": 1}),
        json!({"type": "refused_before_start", "reason": {"type": "future_refusal"}}),
        json!({"type": 1}),
        json!({}),
        json!([]),
    ] {
        assert!(
            serde_json::from_value::<PrepareOutcome>(malformed.clone()).is_err(),
            "{malformed}"
        );
    }
    assert!(serde_json::from_str::<PrepareOutcome>(
        r#"{"type":"refused_before_start","reason":"unreachable","reason":"runner_full"}"#
    )
    .is_err());
    assert!(serde_json::from_str::<PrepareOutcome>(
        r#"{"type":"prepared","type":"future_prepare"}"#
    )
    .is_err());
}

fn string_tags_reject_malformed_shapes<T: DeserializeOwned>(tags: &[&str]) {
    // String enums have no fields: objects or arrays carrying even a known tag
    // are malformed, not a future variant.
    for &tag in tags {
        for malformed in [
            json!({"type": tag}),
            json!({tag: {"future_field": true}}),
            json!([tag]),
        ] {
            assert!(
                serde_json::from_value::<T>(malformed.clone()).is_err(),
                "{malformed}"
            );
        }
    }
    for malformed in [json!(null), json!(7), json!(true)] {
        assert!(
            serde_json::from_value::<T>(malformed.clone()).is_err(),
            "{malformed}"
        );
    }
}

#[test]
fn known_killed_tags_with_malformed_fields_are_rejected() {
    string_tags_reject_malformed_shapes::<Killed>(&["deadline", "cancel"]);
}

#[test]
fn known_ran_tags_with_malformed_fields_are_rejected() {
    string_tags_reject_malformed_shapes::<Ran>(&["remote", "none"]);
}

#[test]
fn known_output_stream_tags_with_malformed_fields_are_rejected() {
    string_tags_reject_malformed_shapes::<OutputStream>(&["stdout", "stderr"]);
}

#[test]
fn known_rebuild_result_tags_with_malformed_fields_are_rejected() {
    string_tags_reject_malformed_shapes::<RebuildResult>(&["building", "ok", "failed"]);
}

#[test]
fn known_caller_tags_keep_their_semantic_variants() {
    for (tag, expected) in [("deadline", Killed::Deadline), ("cancel", Killed::Cancel)] {
        assert_eq!(
            serde_json::from_value::<Killed>(json!(tag)).unwrap(),
            expected
        );
    }
    for (tag, expected) in [("remote", Ran::Remote), ("none", Ran::None)] {
        assert_eq!(serde_json::from_value::<Ran>(json!(tag)).unwrap(), expected);
    }
    for (tag, expected) in [
        ("stdout", OutputStream::Stdout),
        ("stderr", OutputStream::Stderr),
    ] {
        assert_eq!(
            serde_json::from_value::<OutputStream>(json!(tag)).unwrap(),
            expected
        );
    }
    for (tag, expected) in [
        ("building", RebuildResult::Building),
        ("ok", RebuildResult::Ok),
        ("failed", RebuildResult::Failed),
    ] {
        assert_eq!(
            serde_json::from_value::<RebuildResult>(json!(tag)).unwrap(),
            expected
        );
    }
    assert_eq!(
        serde_json::from_value::<PrepareOutcome>(json!({"type": "prepared"})).unwrap(),
        PrepareOutcome::Prepared
    );
    assert_eq!(
        serde_json::from_value::<PrepareOutcome>(
            json!({"type": "refused_before_start", "reason": "unreachable"})
        )
        .unwrap(),
        PrepareOutcome::RefusedBeforeStart {
            reason: RefusalReason::Unreachable
        }
    );
    let case: OutcomeCase = serde_json::from_value(value("outcomes", "exit")).unwrap();
    assert!(matches!(case.stream[0], StreamRecord::Accepted(_)));
    assert!(matches!(case.stream[1], StreamRecord::Terminal(_)));
    let output: StreamRecord = serde_json::from_value(
        json!({"type": "output", "seq": 7, "stream": "stdout", "bytes": "4g=="}),
    )
    .unwrap();
    assert!(matches!(output, StreamRecord::Output(_)));
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
        serde_json::to_value(
            Accepted::new(job_id(), 1).with_env_not_forwarded(vec!["AWS_SECRET_ACCESS_KEY".into()])
        )
        .unwrap(),
        json!({"job_id": job_id(), "queue_position": 1, "env_not_forwarded": ["AWS_SECRET_ACCESS_KEY"]})
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

/// An accepted reply from a runner that predates `env_not_forwarded` decodes
/// as "not reported", and one that carries the list round-trips unchanged.
#[test]
fn accepted_env_not_forwarded_is_optional_and_round_trips() {
    let old: Accepted =
        serde_json::from_value(json!({"job_id": job_id(), "queue_position": 3})).unwrap();
    assert_eq!(old.env_not_forwarded, None);
    let new =
        Accepted::new(job_id(), 3).with_env_not_forwarded(vec!["TOKEN".into(), "HOME".into()]);
    let back: Accepted = serde_json::from_value(serde_json::to_value(&new).unwrap()).unwrap();
    assert_eq!(back, new);
    assert_eq!(
        back.env_not_forwarded.as_deref(),
        Some(&["TOKEN".to_string(), "HOME".to_string()][..])
    );
}

#[test]
fn refusal_hints_match_vectors_and_builders() {
    let terminal = TerminalRecord::new(
        job_id(),
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::RunnerFull,
        },
        0,
        25,
        29,
    )
    .with_ran(Ran::None)
    .with_tree_hash("3333333333333333333333333333333333333333")
    .with_refusal_detail("The runner is at capacity; please retry shortly.")
    .with_retry_after_ms(250);
    assert_eq!(terminal, server_report("crate-local-refusal-hints"));
    assert_eq!(
        terminal.refusal_detail(),
        Some("The runner is at capacity; please retry shortly.")
    );
    assert_eq!(terminal.retry_after_ms(), Some(250));

    let prepare = PrepareReply::new(
        "0192a64a-1234-7000-8000-000000000002".parse().unwrap(),
        PrepareOutcome::RefusedBeforeStart {
            reason: RefusalReason::Unreachable,
        },
    )
    .with_refusal_detail("The runner is temporarily unreachable; please retry shortly.")
    .with_retry_after_ms(500);
    let pinned: PrepareReply =
        serde_json::from_slice(&bytes("replies", "crate-local-prepare-refusal-hints")).unwrap();
    assert_eq!(prepare, pinned);
    assert_eq!(
        prepare.refusal_detail(),
        Some("The runner is temporarily unreachable; please retry shortly.")
    );
    assert_eq!(prepare.retry_after_ms(), Some(500));
}

#[test]
fn refusal_hints_absent_preserve_existing_golden_bytes() {
    let original: OutcomeCase = serde_json::from_slice(&bytes("outcomes", "runner_full")).unwrap();
    let terminal = TerminalRecord::new(
        job_id(),
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::RunnerFull,
        },
        0,
        25,
        29,
    )
    .with_ran(Ran::None)
    .with_tree_hash("3333333333333333333333333333333333333333");
    assert_eq!(terminal.refusal_detail(), None);
    assert_eq!(terminal.retry_after_ms(), None);
    assert_eq!(terminal, server_report("runner_full"));
    let constructed = OutcomeCase {
        request: original.request,
        stream: vec![StreamRecord::Terminal(terminal)],
    };
    assert_eq!(
        serde_json_canonicalizer::to_vec(&constructed).unwrap(),
        bytes("outcomes", "runner_full")
    );
    let mut extended: OutcomeCase =
        serde_json::from_slice(&bytes("outcomes", "crate-local-refusal-hints")).unwrap();
    let [StreamRecord::Terminal(terminal)] = extended.stream.as_mut_slice() else {
        panic!("expected one terminal refusal")
    };
    terminal.refusal_detail = None;
    terminal.retry_after_ms = None;
    assert_eq!(
        serde_json_canonicalizer::to_vec(&extended).unwrap(),
        bytes("outcomes", "runner_full")
    );

    let original: PrepareReply =
        serde_json::from_slice(&bytes("replies", "prepare-unreachable")).unwrap();
    assert_eq!(original.refusal_detail(), None);
    assert_eq!(original.retry_after_ms(), None);
    let constructed = PrepareReply::new(original.transfer_id, original.outcome.clone());
    assert_eq!(constructed, original);
    assert_eq!(
        serde_json_canonicalizer::to_vec(&constructed).unwrap(),
        bytes("replies", "prepare-unreachable")
    );
    let mut extended: PrepareReply =
        serde_json::from_slice(&bytes("replies", "crate-local-prepare-refusal-hints")).unwrap();
    extended.refusal_detail = None;
    extended.retry_after_ms = None;
    assert_eq!(
        serde_json_canonicalizer::to_vec(&extended).unwrap(),
        bytes("replies", "prepare-unreachable")
    );
}

// These containers mirror 0.2.2, before refusal metadata was added. Their field
// types are unchanged; the hint fixtures use reasons already recognised in 0.2.2.
#[derive(Debug, Serialize, Deserialize)]
struct TerminalRecord022 {
    job_id: Uuid,
    outcome: Outcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    killed: Option<Killed>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pipestatus: Option<Vec<i32>>,
    wall_ms: u64,
    queue_wait_ms: u64,
    bundle_bytes: u64,
    ran: Option<Ran>,
    tree_hash: Option<String>,
    workspace_changes: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    git_state_changed: Option<GitStateChange>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    untracked_files: Option<UntrackedFiles>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ignored_writes: Option<IgnoredWrites>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum StreamRecord022 {
    Terminal(TerminalRecord022),
}

#[derive(Debug, Serialize, Deserialize)]
struct OutcomeCase022 {
    request: RunRequest,
    stream: Vec<StreamRecord022>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PrepareReply022 {
    transfer_id: Uuid,
    outcome: PrepareOutcome,
}

#[test]
fn refusal_hints_are_ignored_by_0_2_2_container_decoders() {
    assert_eq!(
        round_trip::<OutcomeCase022>(&bytes("outcomes", "crate-local-refusal-hints")),
        bytes("outcomes", "runner_full")
    );
    assert_eq!(
        round_trip::<PrepareReply022>(&bytes("replies", "crate-local-prepare-refusal-hints")),
        bytes("replies", "prepare-unreachable")
    );
}

#[test]
fn refusal_detail_longer_than_producer_limit_decodes_without_truncation() {
    assert_eq!(REFUSAL_DETAIL_MAX_BYTES, 1024);
    // Multi-byte text makes the producer's byte limit distinct from a character
    // limit. Decoding must keep even an over-limit value verbatim.
    let detail = "é".repeat(REFUSAL_DETAIL_MAX_BYTES / 2 + 1);
    assert_eq!(detail.len(), REFUSAL_DETAIL_MAX_BYTES + 2);
    let mut terminal = value("outcomes", "crate-local-refusal-hints")["stream"][0].clone();
    terminal["refusal_detail"] = json!(detail);
    let decoded: TerminalRecord = serde_json::from_value(terminal).unwrap();
    assert_eq!(decoded.refusal_detail(), Some(detail.as_str()));
    assert_eq!(
        serde_json::to_value(decoded).unwrap()["refusal_detail"],
        detail
    );

    let mut prepare = value("replies", "crate-local-prepare-refusal-hints");
    prepare["refusal_detail"] = json!(detail);
    let decoded: PrepareReply = serde_json::from_value(prepare).unwrap();
    assert_eq!(decoded.refusal_detail(), Some(detail.as_str()));
    assert_eq!(
        serde_json::to_value(decoded).unwrap()["refusal_detail"],
        detail
    );
}

#[test]
fn refusal_hints_are_independently_optional_and_preserve_u64_bounds() {
    let terminal = server_report("runner_full");
    let detail_only = terminal.clone().with_refusal_detail("");
    assert_eq!(detail_only.refusal_detail(), Some(""));
    assert_eq!(detail_only.retry_after_ms(), None);
    let retry_only = terminal.with_retry_after_ms(u64::MAX);
    assert_eq!(retry_only.refusal_detail(), None);
    let decoded: TerminalRecord =
        serde_json::from_value(serde_json::to_value(retry_only).unwrap()).unwrap();
    assert_eq!(decoded.retry_after_ms(), Some(u64::MAX));

    let prepare: PrepareReply =
        serde_json::from_slice(&bytes("replies", "prepare-unreachable")).unwrap();
    let detail_only = prepare.clone().with_refusal_detail("");
    assert_eq!(detail_only.refusal_detail(), Some(""));
    assert_eq!(detail_only.retry_after_ms(), None);
    for hint in [0, u64::MAX] {
        let retry_only = prepare.clone().with_retry_after_ms(hint);
        assert_eq!(retry_only.refusal_detail(), None);
        let decoded: PrepareReply =
            serde_json::from_value(serde_json::to_value(retry_only).unwrap()).unwrap();
        assert_eq!(decoded.retry_after_ms(), Some(hint));
    }
}

#[test]
fn runner_draining_vector_uses_known_reason_and_round_trips_string() {
    let terminal = server_report("crate-local-runner-draining");
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::RunnerDraining
        }
    );
    assert_eq!(
        RefusalReason::from("runner_draining".to_owned()),
        RefusalReason::RunnerDraining
    );
    assert_eq!(
        String::from(RefusalReason::RunnerDraining),
        "runner_draining"
    );
}

#[test]
fn runner_disk_full_vector_uses_known_reason_and_round_trips_string() {
    let terminal = server_report("crate-local-runner-disk-full");
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::RunnerDiskFull
        }
    );
    assert_eq!(
        RefusalReason::from("runner_disk_full".to_owned()),
        RefusalReason::RunnerDiskFull
    );
    assert_eq!(
        String::from(RefusalReason::RunnerDiskFull),
        "runner_disk_full"
    );
}

#[test]
fn network_absent_preserves_existing_golden_bytes() {
    let mut original: OutcomeCase = serde_json::from_slice(&bytes("outcomes", "exit")).unwrap();
    assert_eq!(original.request.network(), None);
    let [StreamRecord::Accepted(accepted), StreamRecord::Terminal(_)] =
        original.stream.as_mut_slice()
    else {
        panic!("expected acceptance followed by a terminal record")
    };
    assert_eq!(accepted.network(), None);

    let request = RunRequest::new(
        "/Users/example/worktrees/task",
        "/Users/example/src/prefrontal",
        "/Users/example/worktrees/task",
        "cargo nextest run -p prefrontal-core-store",
    )
    .with_env([("RUST_BACKTRACE".into(), "1".into())].into())
    .with_queue_wait_limit_s(300)
    .with_siblings(vec!["/Users/example/src/commons".into()])
    .with_timeout(60)
    .with_weight_hint(16);
    assert_eq!(request.network(), None);
    assert_eq!(request, original.request);
    let constructed = Accepted::new(job_id(), 1);
    assert_eq!(constructed.network(), None);
    assert_eq!(&constructed, accepted);
    *accepted = constructed;
    original.request = request;
    assert_eq!(
        serde_json_canonicalizer::to_vec(&original).unwrap(),
        bytes("outcomes", "exit"),
        "offline request and unacknowledged acceptance must preserve the existing exit vector"
    );
}

#[test]
fn network_outbound_vector_matches_known_variant_and_builders() {
    let mut case: OutcomeCase =
        serde_json::from_slice(&bytes("outcomes", "crate-local-network-outbound")).unwrap();
    assert_eq!(case.request.network(), Some(&Network::Outbound));
    let [StreamRecord::Accepted(accepted), StreamRecord::Terminal(_)] = case.stream.as_mut_slice()
    else {
        panic!("expected acceptance followed by a terminal record")
    };
    assert_eq!(accepted.network(), Some(&Network::Outbound));
    assert_eq!(Network::from("outbound".to_owned()), Network::Outbound);
    assert_eq!(String::from(Network::Outbound), "outbound");

    let mut offline: OutcomeCase = serde_json::from_slice(&bytes("outcomes", "exit")).unwrap();
    offline.request = offline.request.with_network(Network::Outbound);
    assert_eq!(offline.request, case.request);
    let constructed = Accepted::new(job_id(), 1).with_network(Network::Outbound);
    assert_eq!(&constructed, accepted);
    offline.stream[0] = StreamRecord::Accepted(constructed);
    assert_eq!(
        serde_json_canonicalizer::to_vec(&offline).unwrap(),
        bytes("outcomes", "crate-local-network-outbound")
    );
}

#[test]
fn network_unknown_vector_retains_string_and_refusal_before_start() {
    let case: OutcomeCase =
        serde_json::from_slice(&bytes("outcomes", "crate-local-unknown-network")).unwrap();
    let unknown = Network::Unknown("future_network".into());
    assert_eq!(case.request.network(), Some(&unknown));
    assert_eq!(Network::from("future_network".to_owned()), unknown);
    assert_eq!(String::from(unknown.clone()), "future_network");
    let [StreamRecord::Terminal(terminal)] = case.stream.as_slice() else {
        panic!("expected refusal without acceptance or start")
    };
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::NetworkUnsupported
        }
    );
    assert_eq!(
        round_trip::<OutcomeCase>(&bytes("outcomes", "crate-local-unknown-network")),
        bytes("outcomes", "crate-local-unknown-network")
    );
    let accepted = Accepted::new(job_id(), 1).with_network(unknown.clone());
    assert_eq!(
        serde_json::from_value::<Accepted>(serde_json::to_value(&accepted).unwrap())
            .unwrap()
            .network(),
        Some(&unknown)
    );
}

#[test]
fn network_unsupported_vector_uses_known_refusal_reason() {
    let terminal = server_report("crate-local-network-unsupported");
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::NetworkUnsupported
        }
    );
    assert_eq!(
        RefusalReason::from("network_unsupported".to_owned()),
        RefusalReason::NetworkUnsupported
    );
    assert_eq!(
        String::from(RefusalReason::NetworkUnsupported),
        "network_unsupported"
    );
}

#[test]
fn network_tags_reject_non_string_shapes() {
    string_tags_reject_malformed_shapes::<Network>(&["outbound", "future_network"]);
}

#[test]
fn network_null_fields_decode_as_absent() {
    let mut request = value("outcomes", "exit")["request"].clone();
    request["network"] = Value::Null;
    assert_eq!(
        serde_json::from_value::<RunRequest>(request)
            .unwrap()
            .network(),
        None
    );
    let accepted: Accepted = serde_json::from_value(json!({
        "job_id": job_id(), "queue_position": 1, "network": null
    }))
    .unwrap();
    assert_eq!(accepted.network(), None);
}

#[test]
fn platform_variants_round_trip_in_requests_and_acknowledgements() {
    for (wire_name, platform) in [
        ("linux", Platform::Linux),
        ("windows", Platform::Windows),
        (
            "future_platform",
            Platform::Unknown("future_platform".into()),
        ),
    ] {
        let request = RunRequest::new("workspace", "/repo", "/workspace", "true")
            .with_platform(platform.clone());
        let request_value = serde_json::to_value(&request).unwrap();
        assert_eq!(request_value["platform"], wire_name);
        assert_eq!(
            serde_json::from_value::<RunRequest>(request_value).unwrap(),
            request
        );

        let accepted = Accepted::new(job_id(), 1).with_platform(platform.clone());
        let accepted_value = serde_json::to_value(&accepted).unwrap();
        assert_eq!(accepted_value["platform"], wire_name);
        assert_eq!(
            serde_json::from_value::<Accepted>(accepted_value).unwrap(),
            accepted
        );
    }
}

#[test]
fn platform_absent_preserves_legacy_request_wire_bytes() {
    let request = RunRequest::new("workspace", "/repo", "/workspace", "true");
    assert_eq!(request.platform(), None);
    let wire = serde_json::to_string(&request).unwrap();
    assert_eq!(
        wire,
        r#"{"workspace_key":"workspace","repository_root":"/repo","cwd":"/workspace","command":"true","env":{},"siblings":[]}"#
    );
    assert_eq!(serde_json::from_str::<RunRequest>(&wire).unwrap(), request);
}

#[test]
fn accepted_without_platform_decodes_as_unacknowledged() {
    let accepted: Accepted = serde_json::from_value(json!({
        "job_id": job_id(), "queue_position": 1
    }))
    .unwrap();
    assert_eq!(accepted.platform(), None);
}

#[test]
fn unknown_platform_is_retained_and_refused_before_start() {
    let case: OutcomeCase =
        serde_json::from_slice(&bytes("outcomes", "crate-local-unknown-platform")).unwrap();
    assert_eq!(
        case.request.platform(),
        Some(&Platform::Unknown("future_platform".into()))
    );
    let [StreamRecord::Terminal(terminal)] = case.stream.as_slice() else {
        panic!("an unknown platform must be refused before acceptance or start")
    };
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::PlatformUnsupported
        }
    );
    assert_eq!(terminal.refusal_detail(), Some("future_platform"));
    assert_eq!(
        round_trip::<OutcomeCase>(&bytes("outcomes", "crate-local-unknown-platform")),
        bytes("outcomes", "crate-local-unknown-platform")
    );
}

#[test]
fn platform_unsupported_refusal_round_trips_requested_platform_name() {
    let case: OutcomeCase =
        serde_json::from_slice(&bytes("outcomes", "crate-local-platform-unsupported")).unwrap();
    assert_eq!(case.request.platform(), Some(&Platform::Windows));
    let [StreamRecord::Terminal(terminal)] = case.stream.as_slice() else {
        panic!("an unsupported platform must be refused before acceptance or start")
    };
    assert_eq!(
        terminal.outcome,
        Outcome::RefusedBeforeStart {
            reason: RefusalReason::PlatformUnsupported
        }
    );
    assert_eq!(terminal.refusal_detail(), Some("windows"));
    assert_eq!(
        RefusalReason::from("platform_unsupported".to_owned()),
        RefusalReason::PlatformUnsupported
    );
    assert_eq!(
        String::from(RefusalReason::PlatformUnsupported),
        "platform_unsupported"
    );
    assert_eq!(
        round_trip::<OutcomeCase>(&bytes("outcomes", "crate-local-platform-unsupported")),
        bytes("outcomes", "crate-local-platform-unsupported")
    );
}

// These shapes mirror 0.2.4 before network requests and grants were added, so
// their re-encoding checks that older decoders ignore only the additive fields.
#[derive(Serialize, Deserialize)]
struct RunRequest024 {
    workspace_key: String,
    repository_root: String,
    cwd: String,
    command: String,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weight_hint: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    queue_wait_limit_s: Option<u64>,
    #[serde(default)]
    siblings: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct Accepted024 {
    job_id: Uuid,
    queue_position: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    env_not_forwarded: Option<Vec<String>>,
}

#[test]
fn network_fields_are_ignored_by_0_2_4_decoders() {
    let extended = value("outcomes", "crate-local-network-outbound");
    let original = value("outcomes", "exit");
    assert_eq!(
        round_trip::<RunRequest024>(&serde_json::to_vec(&extended["request"]).unwrap()),
        serde_json_canonicalizer::to_vec(&original["request"]).unwrap()
    );
    let mut accepted = extended["stream"][0].clone();
    accepted.as_object_mut().unwrap().remove("type");
    let mut old_accepted = original["stream"][0].clone();
    old_accepted.as_object_mut().unwrap().remove("type");
    assert_eq!(
        round_trip::<Accepted024>(&serde_json::to_vec(&accepted).unwrap()),
        serde_json_canonicalizer::to_vec(&old_accepted).unwrap()
    );
}
