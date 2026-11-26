use std::{collections::BTreeSet, fs, path::PathBuf};

use cortexkit_doctor::{
    build_local_report, build_report, CheckOutput, Crash, DecodeError, FindingId, FixOutput,
    Manifest, ModuleId, ReportContext, ReportError, ReportInput, RunError,
};
use serde::Deserialize;
use serde_json::{json, value::RawValue, Value};
use sha2::{Digest, Sha256};

#[derive(Deserialize)]
struct Inputs<'a> {
    report_key: [u8; 32],
    other_report_key: [u8; 32],
    report_time_ms: i64,
    data_home: PathBuf,
    config_home: PathBuf,
    temp_dir: PathBuf,
    title: String,
    description: String,
    cr_tail: String,
    evicted_body_bytes: usize,
    #[serde(borrow)]
    check: &'a RawValue,
    #[serde(borrow)]
    refused_fix: &'a RawValue,
    canaries: Vec<String>,
}

impl Inputs<'_> {
    fn context(&self, key: [u8; 32]) -> ReportContext {
        ReportContext::new(
            key,
            self.report_time_ms,
            &self.data_home,
            &self.config_home,
            &self.temp_dir,
        )
    }
}

fn vector(name: &str) -> Vec<u8> {
    fs::read(vector_dir().join(name)).unwrap()
}

fn vector_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../test-vectors/doctor-v1")
}

fn manifest() -> Manifest {
    Manifest::decode(&vector("manifest.json")).unwrap()
}

fn as_report(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes).unwrap()
}

fn assert_vector(actual: &[u8], name: &str) {
    let expected = vector(name);
    assert_eq!(as_report(actual), as_report(&expected), "{name}");
    assert_eq!(actual, expected, "{name} must use canonical JSON bytes");
}

#[test]
fn report_vectors_checksums_cover_directory() {
    let sums = String::from_utf8(vector("SHA256SUMS")).unwrap();
    let mut listed = BTreeSet::new();
    for entry in sums.lines() {
        let (sha, name) = entry.split_once("  ").unwrap();
        assert!(listed.insert(name.to_owned()), "duplicate: {name}");
        assert_eq!(format!("{:x}", Sha256::digest(vector(name))), sha, "{name}");
    }
    let files = fs::read_dir(vector_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS")
        .collect::<BTreeSet<_>>();
    assert_eq!(listed, files);
}

#[test]
fn report_vectors_canonical_and_canaries() {
    let raw = vector("inputs.json");
    let data: Inputs<'_> = serde_json::from_slice(&raw).unwrap();
    let context = data.context(data.report_key);
    let manifest = manifest();
    assert_eq!(
        manifest.to_canonical_bytes().unwrap(),
        vector("manifest.json")
    );
    let check = CheckOutput::decode(data.check.get().as_bytes()).unwrap();
    let refused = FixOutput::decode(data.refused_fix.get().as_bytes()).unwrap();
    let fixes = [
        (
            FindingId::new("store.note").unwrap(),
            Err(RunError::TimedOut),
        ),
        (FindingId::new("store.backlog").unwrap(), Ok(refused)),
    ];
    let tail_a = String::from_utf8(vector("tail-a.log")).unwrap();
    let tail_b = String::from_utf8(vector("tail-b.log")).unwrap();
    let tails = ["", tail_a.as_str(), tail_b.as_str(), &data.cr_tail];
    let mut input = ReportInput::new(
        manifest.module_id(),
        &manifest,
        &data.title,
        &data.description,
        Ok(&check),
        usize::MAX,
    )
    .with_fix_results(&fixes)
    .with_log_tails(&tails);
    let full = build_report(&context, &input).unwrap();
    assert_vector(&full, "report.json");
    let text = std::str::from_utf8(&full).unwrap();
    for canary in &data.canaries {
        assert!(!text.contains(canary), "canary survived: {canary}");
    }
    assert!(text.contains("taxes-2025.pdf"));

    input.max_body_bytes = data.evicted_body_bytes;
    assert_vector(
        &build_report(&context, &input).unwrap(),
        "evicted-report.json",
    );
    input.max_body_bytes = usize::MAX;
    input.check = Err(RunError::TimedOut);
    input.fix_results = &[];
    input.log_tails = &[];
    assert_vector(
        &build_report(&context, &input).unwrap(),
        "timed-out-check.json",
    );
}

#[test]
fn report_vectors_local_note_without_manifest() {
    let raw = vector("inputs.json");
    let data: Inputs<'_> = serde_json::from_slice(&raw).unwrap();
    let context = data.context(data.report_key);
    let module_id = ModuleId::new("local-example").unwrap();
    let check = CheckOutput::decode(data.check.get().as_bytes()).unwrap();
    let bytes = build_local_report(&context, &module_id, &check).unwrap();

    let expected = json!({
        "schema": "ck.doctor-report/v1",
        "module_id": "local-example",
        "title": "",
        "description": "",
        "check": {"outcome": "ok"},
        "findings": as_report(&vector("report.json"))["findings"],
        "fix_results": [],
        "logs": [],
        "dropped_values": 0,
        "omitted_lines": 0,
        "evicted_lines": 0,
    });
    assert_eq!(as_report(&bytes), expected);
    assert_eq!(bytes, serde_jcs::to_vec(&expected).unwrap());
    let text = std::str::from_utf8(&bytes).unwrap();
    for canary in &data.canaries {
        assert!(!text.contains(canary), "canary survived: {canary}");
    }
}

#[test]
fn report_preserves_roundtrip_precision_of_evidence() {
    let manifest = manifest();
    let check = CheckOutput::decode(
        br#"{
            "schema": "ck.doctor-check/v1",
            "findings": [{
                "id": "store.ratio",
                "severity": "info",
                "summary": "Report the ratio.",
                "fix": {"class": "none", "needs_stop": false},
                "evidence": [{"name": "ratio", "kind": "ratio", "value": 9007199254740993.0}]
            }]
        }"#,
    )
    .unwrap();
    let raw = vector("inputs.json");
    let data: Inputs<'_> = serde_json::from_slice(&raw).unwrap();
    let input = ReportInput::new(
        manifest.module_id(),
        &manifest,
        &data.title,
        &data.description,
        Ok(&check),
        usize::MAX,
    );
    let report = build_report(&data.context(data.report_key), &input).unwrap();
    let text = std::str::from_utf8(&report).unwrap();
    // The input is exactly halfway between consecutive f64 values. Round to even.
    assert!(
        text.contains(r#""kind":"ratio","name":"ratio","value":9007199254740992}"#),
        "{text}"
    );
}

#[test]
fn report_vectors_aliases_change_with_key() {
    let raw = vector("inputs.json");
    let data: Inputs<'_> = serde_json::from_slice(&raw).unwrap();
    let manifest = manifest();
    let tails = ["2024-01-01T00:00:00.000Z INFO  example: check session_id=invented-scope"];
    let input = ReportInput::new(
        manifest.module_id(),
        &manifest,
        "",
        "",
        Err(RunError::TimedOut),
        usize::MAX,
    )
    .with_log_tails(&tails);
    let project = |key| as_report(&build_report(&data.context(key), &input).unwrap());
    let one = project(data.report_key);
    let two = project(data.other_report_key);
    assert_eq!(one["logs"][0]["fields"]["session_id"], "session#9dfad181");
    assert_eq!(two["logs"][0]["fields"]["session_id"], "session#c864d74f");
}

#[test]
fn report_vectors_failure_details_and_module_mismatch() {
    let context = ReportContext::new([0; 32], 0, "/data", "/config", "/tmp");
    let manifest = manifest();
    let id = FindingId::new("finding").unwrap();
    let fixes = [
        (id.clone(), Err(RunError::Crashed(Crash::ExitCode(3)))),
        (id.clone(), Err(RunError::Crashed(Crash::Signal(6)))),
        (id.clone(), Err(RunError::InvalidOutput(DecodeError::Shape))),
        (
            id.clone(),
            Err(RunError::LaunchFailed(std::io::ErrorKind::NotFound)),
        ),
        (id.clone(), Err(RunError::HomeRelative)),
        (id, Err(RunError::NoDoctor)),
    ];
    let mut input = ReportInput::new(
        manifest.module_id(),
        &manifest,
        "",
        "",
        Err(RunError::InvalidOutput(DecodeError::ExtraOutput)),
        usize::MAX,
    )
    .with_fix_results(&fixes);
    let report = as_report(&build_report(&context, &input).unwrap());
    assert_eq!(
        report["check"],
        json!({"outcome":"invalid_output","code":"extra_output"})
    );
    assert_eq!(
        report["fix_results"],
        json!([
            {"finding_id":"finding","outcome":"crashed","exit_code":3,"evidence":[]},
            {"finding_id":"finding","outcome":"crashed","signal":6,"evidence":[]},
            {"finding_id":"finding","outcome":"invalid_output","code":"shape","evidence":[]},
            {"finding_id":"finding","outcome":"launch_failed","evidence":[]},
            {"finding_id":"finding","outcome":"home_relative","evidence":[]},
            {"finding_id":"finding","outcome":"no_doctor","evidence":[]}
        ])
    );

    let other = ModuleId::new("other").unwrap();
    input.module_id = &other;
    input.max_body_bytes = 0;
    assert_eq!(
        build_report(&context, &input),
        Err(ReportError::ModuleMismatch)
    );
}

#[test]
fn report_vectors_budget_counts_extra_digits_without_truncating_findings() {
    let raw = vector("inputs.json");
    let data: Inputs<'_> = serde_json::from_slice(&raw).unwrap();
    let context = data.context(data.report_key);
    let manifest = manifest();
    let check = CheckOutput::decode(data.check.get().as_bytes()).unwrap();
    let mut input = ReportInput::new(
        manifest.module_id(),
        &manifest,
        "",
        "",
        Ok(&check),
        usize::MAX,
    );
    let bare = build_report(&context, &input).unwrap();
    let tail = "2024-01-01T00:00:00.000Z INFO  example: check\n".repeat(10);
    let tails = [tail.as_str()];
    input.log_tails = &tails;
    input.max_body_bytes = bare.len() + 1;
    let empty = build_report(&context, &input).unwrap();
    assert_eq!(empty.len(), input.max_body_bytes);
    let empty = as_report(&empty);
    assert_eq!(empty["evicted_lines"], 10);
    assert_eq!(empty["logs"], json!([]));
    assert_eq!(empty["findings"], as_report(&bare)["findings"]);
    input.max_body_bytes -= 1;
    assert_eq!(build_report(&context, &input), Err(ReportError::TooLarge));
    input.log_tails = &[];
    input.max_body_bytes -= 1;
    assert_eq!(build_report(&context, &input), Err(ReportError::TooLarge));
}
