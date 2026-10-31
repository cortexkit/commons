use cortexkit_exec_remote_types::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::PathBuf};
use uuid::Uuid;

const SYNTAX_VECTORS: &[&str] = &[
    "refuse-not-whole-gib",
    "refuse-below-minimum",
    "refuse-over-ceiling",
];
const TERMINAL_VECTORS: &[&str] = &[
    "terminal-memory",
    "oom-job-limit",
    "oom-host-pressure",
    "oom-host-oom",
    "oom-unknown-cause",
    "terminal-older-shape",
];

#[derive(Debug, Serialize, Deserialize)]
struct LimitCase {
    capabilities: Vec<String>,
    runner_max: u64,
    request: RunRequest,
    expect: LimitExpectation,
}

#[derive(Debug, Serialize, Deserialize)]
struct LimitExpectation {
    error: LimitError,
}

#[derive(Debug, Serialize, Deserialize)]
struct LimitError {
    code: String,
    detail: MemoryCapRefusal,
}

#[derive(Debug, Serialize, Deserialize)]
struct SyntaxCase {
    request: Value,
    expect: SyntaxExpectation,
}

#[derive(Debug, Serialize, Deserialize)]
struct SyntaxExpectation {
    error: SyntaxError,
}

#[derive(Debug, Serialize, Deserialize)]
struct SyntaxError {
    code: String,
    field: String,
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("test-vectors/exec-remote-v1/memory")
}

fn bytes(name: &str) -> Vec<u8> {
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
fn memory_request_is_optional_and_has_exact_wire_bytes() {
    assert_eq!(MEMORY_CAPABILITY, "exec-remote-memory/v1");
    assert_eq!(MEMORY_MAX_MIB, 262_144);
    let original = request();
    assert_eq!(original.memory_mib(), None);
    let old_bytes = br#"{"workspace_key":"example-task","repository_root":"/example/repo","cwd":"/example/repo","command":"printf hello","env":{},"siblings":[]}"#;
    assert_eq!(serde_json::to_vec(&original).unwrap(), old_bytes);
    assert_eq!(
        serde_json::from_slice::<RunRequest>(old_bytes).unwrap(),
        original
    );
    let mut null = serde_json::to_value(&original).unwrap();
    null["memory_mib"] = Value::Null;
    assert_eq!(
        serde_json::from_value::<RunRequest>(null).unwrap(),
        original
    );

    let stated = original.with_memory_mib(8192);
    assert_eq!(stated.memory_mib(), Some(8192));
    assert_eq!(serde_json::to_vec(&stated).unwrap(), br#"{"workspace_key":"example-task","repository_root":"/example/repo","cwd":"/example/repo","command":"printf hello","env":{},"siblings":[],"memory_mib":8192}"#);
    let vector = bytes("request-memory");
    assert_eq!(
        serde_json::from_slice::<RunRequest>(&vector).unwrap(),
        stated
    );
    assert_eq!(serde_json_canonicalizer::to_vec(&stated).unwrap(), vector);
}

fn decode_memory(memory: Value) -> Result<RunRequest, serde_json::Error> {
    let mut raw = serde_json::to_value(request()).unwrap();
    raw["memory_mib"] = memory;
    serde_json::from_value(raw)
}

fn assert_invalid_memory(memory: Value) {
    let error = decode_memory(memory).unwrap_err();
    assert!(error.to_string().contains("memory_mib"), "{error}");
}

#[test]
fn memory_validation_accepts_whole_gib_through_the_ceiling() {
    for memory in (1024..=MEMORY_MAX_MIB).step_by(1024) {
        assert_eq!(
            decode_memory(json!(memory)).unwrap().memory_mib(),
            Some(memory)
        );
    }
}

#[test]
fn memory_validation_rejects_non_whole_gib() {
    for memory in [1025, 1536, 8191, MEMORY_MAX_MIB - 1] {
        assert_invalid_memory(json!(memory));
    }
}

#[test]
fn memory_validation_rejects_below_one_gib() {
    for memory in [0, 1, 512, 1023] {
        assert_invalid_memory(json!(memory));
    }
}

#[test]
fn memory_validation_rejects_above_ceiling() {
    for memory in [MEMORY_MAX_MIB + 1, MEMORY_MAX_MIB + 1024, u64::MAX] {
        assert_invalid_memory(json!(memory));
    }
}

#[test]
fn memory_validation_requires_a_u64_and_names_the_field() {
    for memory in [
        json!(-1),
        json!(1024.5),
        json!("8192"),
        json!(true),
        json!({}),
        json!([]),
    ] {
        assert_invalid_memory(memory);
    }
    let raw = String::from_utf8(serde_json::to_vec(&request().with_memory_mib(8192)).unwrap())
        .unwrap()
        .replace("8192", "18446744073709551616");
    let error = serde_json::from_str::<RunRequest>(&raw).unwrap_err();
    assert!(error.to_string().contains("memory_mib"), "{error}");
}

#[test]
fn syntax_refusal_vectors_name_memory_mib() {
    for name in SYNTAX_VECTORS {
        let case: SyntaxCase = serde_json::from_slice(&bytes(name)).unwrap();
        assert_eq!(case.expect.error.code, "invalid_params", "{name}");
        assert_eq!(case.expect.error.field, "memory_mib", "{name}");
        let error = serde_json::from_value::<RunRequest>(case.request).unwrap_err();
        assert!(
            error.to_string().contains(&case.expect.error.field),
            "{name}: {error}"
        );
    }
}

#[test]
fn runner_limit_refusal_carries_the_numeric_detail() {
    let case: LimitCase = serde_json::from_slice(&bytes("refuse-runner-limit")).unwrap();
    assert!(case.capabilities.iter().any(|cap| cap == MEMORY_CAPABILITY));
    let refusal = check_memory_cap(&case.request, case.runner_max).unwrap_err();
    assert_eq!(case.expect.error.code, "invalid_params");
    assert_eq!(refusal, case.expect.error.detail);
    assert_eq!(refusal.field(), "memory_mib");
    assert_eq!(refusal.max_mib(), Some(16384));
    assert_eq!(
        serde_json::to_value(refusal).unwrap(),
        json!({"field":"memory_mib","max_mib":16384})
    );
    let raw: Value = serde_json::from_slice(&bytes("refuse-runner-limit")).unwrap();
    assert!(raw["expect"]["error"]["detail"]["max_mib"].is_u64());
    assert!(raw["expect"]["error"].get("job_id").is_none());
}

#[test]
fn memory_cap_check_preserves_absence_and_never_clamps() {
    assert_eq!(check_memory_cap(&request(), 0), Ok(None));
    for (memory, limit) in [(1024, 1024), (16384, 16384), (MEMORY_MAX_MIB, u64::MAX)] {
        assert_eq!(
            check_memory_cap(&request().with_memory_mib(memory), limit),
            Ok(Some(memory))
        );
    }
    for (memory, limit, maximum) in [
        (1024, 0, 0),
        (16384, 8192, 8192),
        (MEMORY_MAX_MIB + 1024, u64::MAX, MEMORY_MAX_MIB),
        (u64::MAX, u64::MAX, MEMORY_MAX_MIB),
    ] {
        assert_eq!(
            check_memory_cap(&request().with_memory_mib(memory), limit),
            Err(MemoryCapRefusal::new(maximum))
        );
    }
    let unstated: MemoryCapRefusal =
        serde_json::from_value(json!({"field":"memory_mib","future":true})).unwrap();
    assert_eq!(unstated.field(), "memory_mib");
    assert_eq!(unstated.max_mib(), None);
    assert_eq!(
        serde_json::to_value(unstated).unwrap(),
        json!({"field":"memory_mib"})
    );
    let future: MemoryCapRefusal =
        serde_json::from_value(json!({"field":"memory_mib","max_mib":8192,"future":true})).unwrap();
    assert_eq!(future, MemoryCapRefusal::new(8192));
}

fn terminal() -> TerminalRecord {
    TerminalRecord::new(Uuid::from_u128(1), Outcome::Exit { code: 0 }, 1000, 0, 0)
        .with_ran(Ran::Remote)
}

#[test]
fn terminal_memory_fields_round_trip_independently() {
    for measurement in [0, 8192, u64::MAX] {
        for (record, field) in [
            (
                terminal().with_memory_stated_mib(measurement),
                "memory_stated_mib",
            ),
            (
                terminal().with_memory_peak_working_mib(measurement),
                "memory_peak_working_mib",
            ),
            (terminal().with_memory_over_s(measurement), "memory_over_s"),
        ] {
            let mut expected = serde_json::to_value(terminal()).unwrap();
            expected[field] = json!(measurement);
            assert_eq!(serde_json::to_value(&record).unwrap(), expected);
            assert_eq!(
                serde_json::from_value::<TerminalRecord>(expected).unwrap(),
                record
            );
        }
    }
    let record = terminal()
        .with_memory_stated_mib(8192)
        .with_memory_peak_working_mib(9000)
        .with_memory_over_s(3);
    assert_eq!(record.memory_stated_mib(), Some(8192));
    assert_eq!(record.memory_peak_working_mib(), Some(9000));
    assert_eq!(record.memory_over_s(), Some(3));
    assert_eq!(record.hosted(), None);
    assert_eq!(
        serde_json::from_slice::<StreamRecord>(&bytes("terminal-memory")).unwrap(),
        StreamRecord::Terminal(record)
    );
}

#[test]
fn oom_kill_named_and_unknown_causes_are_retained() {
    assert_eq!(OOM_KILL_CAUSE_JOB_LIMIT, "job_limit");
    assert_eq!(OOM_KILL_CAUSE_HOST_PRESSURE, "host_pressure");
    assert_eq!(OOM_KILL_CAUSE_HOST_OOM, "host_oom");
    for (name, cause) in [
        ("oom-job-limit", OOM_KILL_CAUSE_JOB_LIMIT),
        ("oom-host-pressure", OOM_KILL_CAUSE_HOST_PRESSURE),
        ("oom-host-oom", OOM_KILL_CAUSE_HOST_OOM),
        ("oom-unknown-cause", "future_memory_cause"),
    ] {
        let kill = OomKill::new(cause);
        assert_eq!(kill.cause(), cause);
        assert_eq!(serde_json::to_value(&kill).unwrap(), json!({"cause":cause}));
        let record = TerminalRecord::new(
            Uuid::from_u128(1),
            Outcome::Signal { signal: 9 },
            1000,
            0,
            0,
        )
        .with_ran(Ran::Remote)
        .with_oom_kill(kill.clone());
        assert_eq!(record.oom_kill(), Some(&kill));
        assert_eq!(record.killed, None);
        let raw = serde_json::to_value(&record).unwrap();
        assert_eq!(raw["oom_kill"], json!({"cause":cause}));
        assert_eq!(
            serde_json::from_value::<TerminalRecord>(raw).unwrap(),
            record
        );
        assert_eq!(
            serde_json::from_slice::<StreamRecord>(&bytes(name)).unwrap(),
            StreamRecord::Terminal(record)
        );
    }
    for cause in ["", "new cause: \u{03bb}"] {
        let raw = json!({"cause":cause,"future":true});
        let decoded: OomKill = serde_json::from_value(raw).unwrap();
        assert_eq!(decoded.cause(), cause);
        assert_eq!(
            serde_json::to_value(decoded).unwrap(),
            json!({"cause":cause})
        );
    }
}

#[test]
fn older_terminal_shape_decodes_unchanged_and_omits_memory_fields() {
    let old_bytes = bytes("terminal-older-shape");
    let decoded: StreamRecord = serde_json::from_slice(&old_bytes).unwrap();
    assert_eq!(decoded, StreamRecord::Terminal(terminal()));
    let StreamRecord::Terminal(record) = &decoded else {
        panic!("expected a terminal record");
    };
    assert_eq!(record.memory_stated_mib(), None);
    assert_eq!(record.memory_peak_working_mib(), None);
    assert_eq!(record.memory_over_s(), None);
    assert_eq!(record.oom_kill(), None);
    assert_eq!(
        serde_json_canonicalizer::to_vec(&decoded).unwrap(),
        old_bytes
    );
}

#[test]
fn memory_vectors_have_exact_inventory_canonical_bytes_and_checksums() {
    let names: Vec<_> = SYNTAX_VECTORS
        .iter()
        .chain(TERMINAL_VECTORS)
        .copied()
        .chain(["request-memory", "refuse-runner-limit"])
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
        let canonical = match name {
            "request-memory" => serde_json_canonicalizer::to_vec(
                &serde_json::from_slice::<RunRequest>(&bytes).unwrap(),
            )
            .unwrap(),
            "refuse-runner-limit" => serde_json_canonicalizer::to_vec(
                &serde_json::from_slice::<LimitCase>(&bytes).unwrap(),
            )
            .unwrap(),
            name if SYNTAX_VECTORS.contains(&name) => serde_json_canonicalizer::to_vec(
                &serde_json::from_slice::<SyntaxCase>(&bytes).unwrap(),
            )
            .unwrap(),
            _ => serde_json_canonicalizer::to_vec(
                &serde_json::from_slice::<StreamRecord>(&bytes).unwrap(),
            )
            .unwrap(),
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
