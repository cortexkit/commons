use cortexkit_exec_remote_types::*;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};

const VALID_RECORDS: &[&str] = &[
    "upload",
    "placement-wait",
    "connect",
    "unknown-phase",
    "no-detail",
    "detail-120-bytes",
];
const INVALID_RECORDS: &[&str] = &[
    "malformed-detail-too-long",
    "malformed-detail-newline",
    "malformed-phase-elapsed-ms",
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/exec-remote-v1/progress")
}

fn vector(name: &str) -> Vec<u8> {
    fs::read(root().join(format!("{name}.jcs"))).unwrap()
}

fn request() -> RunRequest {
    RunRequest::new(
        "example-task",
        "/example/repo",
        "/example/repo",
        "printf hello",
    )
}

#[test]
fn progress_request_is_opt_in_and_false_preserves_existing_bytes() {
    assert_eq!(PROGRESS_CAPABILITY, "exec-remote-progress/v1");
    let original = request();
    assert!(!original.progress());
    let bytes = serde_json::to_vec(&original).unwrap();
    assert_eq!(bytes, br#"{"workspace_key":"example-task","repository_root":"/example/repo","cwd":"/example/repo","command":"printf hello","env":{},"siblings":[]}"#);
    assert_eq!(
        serde_json::to_vec(&original.clone().with_progress(false)).unwrap(),
        bytes
    );
    let mut explicit_false = serde_json::to_value(&original).unwrap();
    explicit_false["progress"] = json!(false);
    let decoded: RunRequest = serde_json::from_value(explicit_false).unwrap();
    assert!(!decoded.progress());
    assert_eq!(serde_json::to_vec(&decoded).unwrap(), bytes);
    assert_eq!(
        serde_json::from_slice::<RunRequest>(&bytes).unwrap(),
        original
    );

    let opted_in = original.with_progress(true);
    assert!(opted_in.progress());
    assert_eq!(serde_json::to_value(&opted_in).unwrap()["progress"], true);
    assert_eq!(
        serde_json::from_slice::<RunRequest>(&vector("request-progress")).unwrap(),
        opted_in
    );
    assert!(!opted_in.with_progress(false).progress());
}

#[test]
fn progress_phases_preserve_known_and_future_tags() {
    for (tag, expected) in [
        ("upload", ProgressPhase::Upload),
        ("placement_wait", ProgressPhase::PlacementWait),
        ("connect", ProgressPhase::Connect),
        (
            "future_phase",
            ProgressPhase::Unknown("future_phase".into()),
        ),
    ] {
        assert_eq!(ProgressPhase::from(tag.to_owned()), expected);
        assert_eq!(String::from(expected.clone()), tag);
        assert_eq!(serde_json::to_value(&expected).unwrap(), json!(tag));
        assert_eq!(
            serde_json::from_value::<ProgressPhase>(json!(tag)).unwrap(),
            expected
        );
    }
    for invalid in [json!(null), json!(3), json!({"type":"upload"}), json!([])] {
        assert!(serde_json::from_value::<ProgressPhase>(invalid).is_err());
    }
}

#[test]
fn progress_accessors_and_wire_shape_have_no_sequence() {
    let progress = Progress::new(
        ProgressPhase::PlacementWait,
        61200,
        Some("waiting for a host with 16 free vCPUs".into()),
    )
    .unwrap();
    assert_eq!(progress.phase(), &ProgressPhase::PlacementWait);
    assert_eq!(progress.phase_elapsed_ms(), 61200);
    assert_eq!(
        progress.detail(),
        Some("waiting for a host with 16 free vCPUs")
    );
    let record = StreamRecord::Progress(progress);
    assert_eq!(
        serde_json::to_value(&record).unwrap(),
        json!({"type":"progress", "phase":"placement_wait", "phase_elapsed_ms":61200,
            "detail":"waiting for a host with 16 free vCPUs"})
    );
    assert_eq!(
        serde_json::from_slice::<StreamRecord>(&vector("placement-wait")).unwrap(),
        record
    );
    let absent = Progress::new(ProgressPhase::Upload, 0, None).unwrap();
    assert_eq!(absent.detail(), None);
    assert!(serde_json::to_value(absent)
        .unwrap()
        .get("detail")
        .is_none());
}

#[test]
fn progress_detail_limit_is_utf8_bytes_on_construction_and_decode() {
    for detail in [
        String::new(),
        "x".repeat(120),
        "é".repeat(60),
        "<b>waiting</b>".into(),
    ] {
        let progress = Progress::new(ProgressPhase::Connect, 0, Some(detail.clone())).unwrap();
        assert_eq!(progress.detail(), Some(detail.as_str()));
        assert_eq!(
            serde_json::from_value::<Progress>(json!({
                "phase":"connect", "phase_elapsed_ms":0, "detail":detail
            }))
            .unwrap(),
            progress
        );
    }
    for detail in ["x".repeat(121), format!("{}x", "é".repeat(60))] {
        assert_invalid_detail(detail);
    }
    let valid: Value = serde_json::from_slice(&vector("detail-120-bytes")).unwrap();
    let invalid: Value = serde_json::from_slice(&vector("malformed-detail-too-long")).unwrap();
    assert_eq!(valid["detail"].as_str().unwrap().len(), 120);
    assert_eq!(invalid["detail"].as_str().unwrap().len(), 121);
}

fn assert_invalid_detail(detail: String) {
    let error = Progress::new(ProgressPhase::Upload, 0, Some(detail.clone())).unwrap_err();
    assert!(error.to_string().contains("120 bytes"));
    let raw = json!({"type":"progress", "phase":"upload", "phase_elapsed_ms":0, "detail":detail});
    assert!(serde_json::from_value::<Progress>(raw.clone()).is_err());
    let error = serde_json::from_value::<StreamRecord>(raw).unwrap_err();
    assert!(error
        .to_string()
        .contains("known stream record tag progress"));
}

#[test]
fn progress_detail_rejects_controls_and_unicode_line_breaks() {
    for character in (0..=0x1f).chain(0x7f..=0x9f).chain([0x2028, 0x2029]) {
        let character = char::from_u32(character).unwrap();
        assert_invalid_detail(format!("waiting{character}for capacity"));
    }
}

#[test]
fn progress_phase_elapsed_ms_requires_a_u64() {
    for elapsed in [0, u64::MAX] {
        let progress: Progress = serde_json::from_value(json!({
            "phase":"upload", "phase_elapsed_ms":elapsed
        }))
        .unwrap();
        assert_eq!(progress.phase_elapsed_ms(), elapsed);
    }
    for invalid in [
        json!(null),
        json!(-1),
        json!(1.5),
        json!("2100"),
        json!(true),
    ] {
        assert!(serde_json::from_value::<StreamRecord>(json!({
            "type":"progress", "phase":"upload", "phase_elapsed_ms":invalid
        }))
        .is_err());
    }
    assert!(serde_json::from_str::<StreamRecord>(
        r#"{"type":"progress","phase":"upload","phase_elapsed_ms":18446744073709551616}"#
    )
    .is_err());
    for missing in [
        json!({"type":"progress", "phase":"upload"}),
        json!({"type":"progress", "phase_elapsed_ms":0}),
        json!({"type":"progress", "phase":"upload", "elapsed_ms":2100}),
    ] {
        assert!(serde_json::from_value::<StreamRecord>(missing).is_err());
    }
}

#[test]
fn progress_ignores_unknown_fields_but_rejects_non_string_detail() {
    let record = StreamRecord::Progress(Progress::new(ProgressPhase::Connect, 2100, None).unwrap());
    let mut raw = serde_json::to_value(&record).unwrap();
    raw["future_field"] = json!({"extra":true});
    assert_eq!(
        serde_json::from_value::<StreamRecord>(raw.clone()).unwrap(),
        record
    );
    raw["detail"] = json!(123);
    assert!(serde_json::from_value::<StreamRecord>(raw).is_err());
}

#[test]
fn malformed_progress_vectors_never_decode_as_unknown() {
    for name in INVALID_RECORDS {
        let bytes = vector(name);
        assert!(
            serde_json::from_slice::<Progress>(&bytes).is_err(),
            "{name}"
        );
        let error = serde_json::from_slice::<StreamRecord>(&bytes).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("known stream record tag progress"),
            "{name}: {error}"
        );
    }
}

#[test]
fn valid_progress_vectors_round_trip_byte_for_byte() {
    for name in VALID_RECORDS {
        let bytes = vector(name);
        let record: StreamRecord = serde_json::from_slice(&bytes).unwrap();
        assert!(matches!(&record, StreamRecord::Progress(_)), "{name}");
        assert_eq!(
            serde_json_canonicalizer::to_vec(&record).unwrap(),
            bytes,
            "{name}"
        );
    }
    let bytes = vector("request-progress");
    let request: RunRequest = serde_json::from_slice(&bytes).unwrap();
    assert!(request.progress());
    assert_eq!(serde_json_canonicalizer::to_vec(&request).unwrap(), bytes);
}

#[test]
fn progress_vector_inventory_and_hashes_are_pinned() {
    let names = VALID_RECORDS
        .iter()
        .chain(INVALID_RECORDS)
        .copied()
        .chain(["request-progress"]);
    let mut expected_files: BTreeSet<_> = names
        .clone()
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
        let bytes = vector(name);
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
