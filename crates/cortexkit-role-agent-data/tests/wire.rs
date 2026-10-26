//! Checks every vector in `test-vectors/agent-data-v1/` against the types:
//! canonical vectors round-trip to the same RFC 8785 (JCS) bytes, refused
//! vectors are refused with the code the contract names, and SHA256SUMS
//! pins every file.

use cortexkit_role_agent_data::{
    abort::AbortReply,
    activate::ActivateReply,
    busy::BusyReply,
    describe::DescribeReply,
    diverged::DivergedReply,
    errors::{
        allowed_refusals, driver_action_for, may_refuse, DriverAction, ErrorBody, RefusalCode,
    },
    export::ExportReply,
    idempotency::{compare, Repeat},
    import::ImportReply,
    prepare::PrepareReply,
    refs::{is_session_ref, Refs},
    release::ReleaseReply,
    request::Request,
    status::StatusReply,
    step::{advance, check_ref_set, Step, StepRefusal},
    Op,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Every vector file. A test checks this list against the directory, so a
/// new file cannot land without being read here and pinned in SHA256SUMS.
const FILES: &[&str] = &[
    "requests.json",
    "replies.json",
    "refusals.json",
    "idempotency.json",
    "steps.json",
    "ref-sets.json",
    "op-refusals.json",
    "session-refs.json",
    "README.md",
    "digest-mismatch.json",
    "diverged-reasons.json",
    "import-drops.json",
];

fn dir() -> String {
    format!("{}/test-vectors/agent-data-v1", env!("CARGO_MANIFEST_DIR"))
}

fn load(name: &str) -> Value {
    assert!(FILES.contains(&name), "{name} is not listed in FILES");
    let path = format!("{}/{name}", dir());
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn cases<'a>(file: &'a Value, key: &str) -> &'a Vec<Value> {
    file[key]
        .as_array()
        .unwrap_or_else(|| panic!("vector key {key} is not an array"))
}

fn name(case: &Value) -> &str {
    case["name"].as_str().expect("every case has a name")
}

/// The RFC 8785 canonical bytes of `value`, from an implementation
/// independent of this crate's encoding.
fn jcs(value: &Value) -> String {
    serde_json_canonicalizer::to_string(value).expect("canonicalizes")
}

/// The case's params, with `refs` replaced by generated refs when the case
/// has an `expand_refs` count (see the vectors' README).
fn params(case: &Value) -> Value {
    let mut params = case["params"].clone();
    if let Some(n) = case.get("expand_refs") {
        let n = n.as_u64().unwrap();
        params["refs"] = Value::Array(
            (0..n)
                .map(|i| json!({"ref_kind": "broca.session", "ref": format!("ses_{i}")}))
                .collect(),
        );
    }
    params
}

/// Decodes `json` as `T` and checks that encoding it gives back the same
/// canonical bytes.
fn round_trip<T>(name: &str, json: &Value) -> T
where
    T: DeserializeOwned + Serialize + PartialEq + std::fmt::Debug,
{
    let decoded: T = serde_json::from_value(json.clone())
        .unwrap_or_else(|e| panic!("{name}: does not decode: {e}"));
    let encoded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(
        jcs(&encoded),
        jcs(json),
        "{name}: encoding differs from the vector"
    );
    let again: T = serde_json::from_value(encoded).unwrap();
    assert_eq!(again, decoded, "{name}: second decode differs");
    decoded
}

fn reply_round_trip(name: &str, method: &str, reply: &Value) {
    match Op::from_method(method).unwrap_or_else(|| panic!("{name}: method {method}")) {
        Op::Describe => drop(round_trip::<DescribeReply>(name, reply)),
        Op::Busy => drop(round_trip::<BusyReply>(name, reply)),
        Op::Status => drop(round_trip::<StatusReply>(name, reply)),
        Op::Prepare => drop(round_trip::<PrepareReply>(name, reply)),
        Op::Export => drop(round_trip::<ExportReply>(name, reply)),
        Op::Diverged => drop(round_trip::<DivergedReply>(name, reply)),
        Op::Import => drop(round_trip::<ImportReply>(name, reply)),
        Op::Activate => drop(round_trip::<ActivateReply>(name, reply)),
        Op::Abort => drop(round_trip::<AbortReply>(name, reply)),
        Op::Release => drop(round_trip::<ReleaseReply>(name, reply)),
    }
}

fn reply_decodes(method: &str, reply: &Value) -> bool {
    fn ok<T: DeserializeOwned>(reply: &Value) -> bool {
        serde_json::from_value::<T>(reply.clone()).is_ok()
    }
    match Op::from_method(method).unwrap() {
        Op::Describe => ok::<DescribeReply>(reply),
        Op::Busy => ok::<BusyReply>(reply),
        Op::Status => ok::<StatusReply>(reply),
        Op::Prepare => ok::<PrepareReply>(reply),
        Op::Export => ok::<ExportReply>(reply),
        Op::Diverged => ok::<DivergedReply>(reply),
        Op::Import => ok::<ImportReply>(reply),
        Op::Activate => ok::<ActivateReply>(reply),
        Op::Abort => ok::<AbortReply>(reply),
        Op::Release => ok::<ReleaseReply>(reply),
    }
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn every_file_on_disk_is_listed() {
    let mut on_disk: Vec<String> = std::fs::read_dir(dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS")
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = FILES.iter().map(|s| s.to_string()).collect();
    listed.sort();
    assert_eq!(on_disk, listed);
}

#[test]
fn files_match_sha256sums() {
    let path = format!("{}/SHA256SUMS", dir());
    let sums = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let checksums: BTreeMap<&str, &str> = sums
        .lines()
        .map(|line| {
            let (hash, name) = line
                .split_once("  ")
                .unwrap_or_else(|| panic!("invalid checksum entry: {line}"));
            (name, hash)
        })
        .collect();
    let listed: BTreeSet<&str> = FILES.iter().copied().collect();
    assert_eq!(
        checksums.keys().copied().collect::<BTreeSet<_>>(),
        listed,
        "SHA256SUMS must list exactly the vector files"
    );
    for name in FILES {
        let path = format!("{}/{name}", dir());
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(sha256(&bytes), checksums[name], "{name}: SHA-256 mismatch");
    }
}

#[test]
fn every_request_round_trips() {
    let file = load("requests.json");
    let mut ops = BTreeSet::new();
    for case in cases(&file, "requests") {
        let name = name(case);
        let method = case["method"].as_str().unwrap();
        let params = params(case);
        let request =
            Request::decode(method, &params).unwrap_or_else(|e| panic!("{name}: refused: {e:?}"));
        assert_eq!(request.op().method(), method, "{name}");
        assert_eq!(
            jcs(&request.params()),
            jcs(&params),
            "{name}: encoding differs from the vector"
        );
        assert_eq!(
            Request::decode(method, &request.params()).unwrap(),
            request,
            "{name}: second decode differs"
        );
        assert_eq!(request.cut_id().is_some(), request.op().carries_cut_id());
        ops.insert(method.to_owned());
    }
    let all: BTreeSet<String> = Op::ALL.iter().map(|op| op.method().to_owned()).collect();
    assert_eq!(ops, all, "every op has a valid request vector");
}

#[test]
fn every_refused_request_is_invalid_request() {
    let file = load("requests.json");
    let refused = cases(&file, "refused");
    for case in refused {
        let name = name(case);
        let method = case["method"].as_str().unwrap();
        let expected = RefusalCode::from_code(case["code"].as_str().unwrap());
        assert_eq!(expected, RefusalCode::InvalidRequest, "{name}");
        match Request::decode(method, &params(case)) {
            Ok(request) => panic!("{name}: decoded, expected a refusal: {request:?}"),
            Err(body) => assert_eq!(body.code, expected, "{name}: {}", body.message),
        }
    }
    for required in ["unknown top-level field", "257 refs", "uppercase cutId"] {
        assert!(
            refused.iter().any(|case| name(case) == required),
            "missing refused case {required:?}"
        );
    }
}

#[test]
fn every_reply_round_trips() {
    let file = load("replies.json");
    let mut ops = BTreeSet::new();
    for case in cases(&file, "replies") {
        let method = case["method"].as_str().unwrap();
        reply_round_trip(name(case), method, &case["reply"]);
        ops.insert(method.to_owned());
    }
    let all: BTreeSet<String> = Op::ALL.iter().map(|op| op.method().to_owned()).collect();
    assert_eq!(ops, all, "every op has a valid reply vector");
}

#[test]
fn every_refused_reply_does_not_decode() {
    let file = load("replies.json");
    for case in cases(&file, "refused") {
        let method = case["method"].as_str().unwrap();
        assert!(
            !reply_decodes(method, &case["reply"]),
            "{}: decoded, expected a refusal",
            name(case)
        );
    }
}

#[test]
fn import_dropped_vectors_validate_and_name_each_refused_rule() {
    let file = load("import-drops.json");
    let valid = cases(&file, "valid");
    assert_eq!(valid.len(), 1);
    for case in valid {
        let _: ImportReply = round_trip(name(case), &case["reply"]);
    }

    let refused = cases(&file, "refused");
    assert_eq!(refused.len(), 14);
    let rules: BTreeSet<&str> = refused
        .iter()
        .map(|case| case["rule"].as_str().unwrap())
        .collect();
    assert_eq!(
        rules,
        BTreeSet::from([
            "dropped must be non-empty",
            "dropped must have at most 16 reasons",
            "dropped reason must be non-empty",
            "dropped reason must be at most 64 ASCII bytes",
            "dropped reason must start with a lowercase ASCII letter",
            "dropped reason must contain only lowercase ASCII letters, digits and `_`",
            "dropped count must be positive",
            "dropped count must be at most 9007199254740991",
            "u64",
        ])
    );
    for case in refused {
        let case_name = name(case);
        let err = serde_json::from_value::<ImportReply>(case["reply"].clone())
            .expect_err(case_name)
            .to_string();
        let rule = case["rule"].as_str().unwrap();
        assert!(
            err.contains(rule),
            "{case_name}: expected error naming {rule:?}, got {err:?}"
        );
    }
}

#[test]
fn import_dropped_absence_preserves_existing_bytes_and_strict_items() {
    let file = load("replies.json");
    let case = cases(&file, "replies")
        .iter()
        .find(|case| name(case) == "import")
        .unwrap();
    let reply: ImportReply = round_trip(name(case), &case["reply"]);
    // Pin the original serializer's bytes, not just a round trip of new bytes.
    assert_eq!(
        serde_json::to_string(&reply).unwrap(),
        concat!(
            "{\"items\":[{\"sessionRef\":\"c6ea741d409f833ee60be91d9176e493\",\"endOffset\":48213,",
            "\"digest\":\"da267b5c82498c6ac578d48420db33f0e814aef902b86d332fc266dc57da05d8\"},",
            "{\"sessionRef\":\"4ace44c6dd17c1d192db9a52d670ed42\",\"endOffset\":0,",
            "\"digest\":\"e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\"}]}"
        )
    );

    let mut null = case["reply"].clone();
    null["items"][0]["dropped"] = Value::Null;
    let decoded: ImportReply = serde_json::from_value(null).unwrap();
    assert_eq!(decoded, reply, "null decodes as absent");

    let mut unknown = case["reply"].clone();
    unknown["items"][0]["future"] = json!(true);
    assert!(serde_json::from_value::<ImportReply>(unknown)
        .unwrap_err()
        .to_string()
        .contains("unknown field `future`"));
}

#[test]
fn diverged_reason_vectors_validate_and_name_each_refused_rule() {
    use cortexkit_role_agent_data::diverged::{DivergedEntry, DivergedState, REASON_MAX_BYTES};

    let file = load("diverged-reasons.json");
    let valid = cases(&file, "valid");
    assert_eq!(
        valid.len(),
        2,
        "one reason vector for each reportable state"
    );
    let states: BTreeSet<&str> = valid
        .iter()
        .map(|case| case["reply"]["refs"][0]["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, BTreeSet::from(["unknown", "advanced"]));
    for case in valid {
        let _: DivergedReply = round_trip(name(case), &case["reply"]);
    }
    let refused = cases(&file, "refused");
    let rules: BTreeSet<&str> = refused
        .iter()
        .map(|case| case["rule"].as_str().expect("each refusal names its rule"))
        .collect();
    assert_eq!(
        rules,
        BTreeSet::from([
            "at_cut",
            "non-empty",
            "256 UTF-8 bytes",
            "control characters"
        ])
    );
    for case in refused {
        let case_name = name(case);
        let err = serde_json::from_value::<DivergedReply>(case["reply"].clone())
            .expect_err(case_name)
            .to_string();
        let rule = case["rule"].as_str().expect("each refusal names its rule");
        assert!(
            err.contains(rule),
            "{case_name}: expected error naming {rule:?}, got {err:?}"
        );
    }

    let null_reason: DivergedReply = serde_json::from_value(json!({
        "refs": [{"state": "unknown", "reason": null}]
    }))
    .expect("null reason decodes as absent");
    assert_eq!(null_reason.refs[0].reason, None);
    assert_eq!(
        serde_json::to_value(null_reason).unwrap(),
        json!({"refs": [{"state": "unknown"}]})
    );

    assert_eq!(DivergedEntry::new(DivergedState::Unknown).reason, None);
    assert!(DivergedEntry::with_reason(DivergedState::Unknown, "cause").is_ok());
    assert!(
        DivergedEntry::with_reason(DivergedState::Unknown, "é".repeat(REASON_MAX_BYTES / 2))
            .is_ok()
    );
    assert!(DivergedEntry::with_reason(
        DivergedState::Unknown,
        "é".repeat(REASON_MAX_BYTES / 2 + 1)
    )
    .unwrap_err()
    .contains("256 UTF-8 bytes"));
    assert!(DivergedEntry::with_reason(DivergedState::AtCut, "cause")
        .unwrap_err()
        .contains("at_cut"));
}

fn driver(action: &str) -> DriverAction {
    match action {
        "abort" => DriverAction::Abort,
        "retry_within_step" => DriverAction::RetryWithinStep,
        "retry_later" => DriverAction::RetryLater,
        "retry_after_operator" => DriverAction::RetryAfterOperator,
        "retry" => DriverAction::Retry,
        "terminal_for_step" => DriverAction::TerminalForStep,
        other => panic!("unknown driver action {other}"),
    }
}

#[test]
fn every_refusal_code_round_trips_with_its_driver_action() {
    let file = load("refusals.json");
    let mut known = BTreeSet::new();
    for case in cases(&file, "refusals") {
        let name = name(case);
        let body: ErrorBody = round_trip(name, &case["body"]);
        assert_eq!(body.code.as_str(), case["body"]["code"], "{name}");
        assert_eq!(
            body.code.is_known(),
            case["named"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(
            body.code.driver_action(),
            driver(case["driver"].as_str().unwrap()),
            "{name}"
        );
        assert_eq!(
            body.code.retryable(),
            case["retryable"].as_bool().unwrap(),
            "{name}"
        );
        if let Some(op) = case.get("op") {
            let op = Op::from_method(&format!("agent_data.{}", op.as_str().unwrap())).unwrap();
            assert!(
                may_refuse(op, &body.code),
                "{name}: {op:?} may not refuse this"
            );
        }
        let detail = body
            .typed_detail()
            .unwrap_or_else(|e| panic!("{name}: detail does not decode: {e}"));
        assert_eq!(detail.is_some(), body.detail.is_some(), "{name}");
        if let Some(detail) = detail {
            let (code, value) = detail.into_parts();
            assert_eq!(code, body.code, "{name}");
            assert_eq!(jcs(&value), jcs(body.detail.as_ref().unwrap()), "{name}");
        }
        if body.code.is_known() {
            known.insert(body.code.as_str().to_owned());
        } else {
            assert_eq!(body.code, RefusalCode::Unknown(body.code.as_str().into()));
        }
    }
    let all: BTreeSet<String> = RefusalCode::KNOWN
        .iter()
        .map(|code| code.as_str().to_owned())
        .collect();
    assert_eq!(known, all, "every refusal code has a vector");
}

#[test]
fn digest_mismatch_refusal_vectors_pin_per_op_actions() {
    let file = load("digest-mismatch.json");
    let refusals = cases(&file, "refusals");
    assert_eq!(refusals.len(), 2);
    for (case, op) in refusals.iter().zip([Op::Activate, Op::Abort]) {
        let name = name(case);
        let body: ErrorBody = round_trip(name, &case["body"]);
        assert_eq!(body.code, RefusalCode::DigestMismatch, "{name}");
        assert_eq!(
            body.code.is_known(),
            case["named"].as_bool().unwrap(),
            "{name}"
        );
        assert_eq!(
            format!("agent_data.{}", case["op"].as_str().unwrap()),
            op.method()
        );
        assert!(may_refuse(op, &body.code), "{name}");
        let action = driver_action_for(op, &body.code);
        assert_eq!(action, driver(case["driver"].as_str().unwrap()), "{name}");
        assert_eq!(
            action.retryable(),
            case["retryable"].as_bool().unwrap(),
            "{name}"
        );
        let detail = body
            .digest_mismatch_detail()
            .unwrap()
            .expect("vector has detail");
        assert_eq!(
            jcs(&serde_json::to_value(detail).unwrap()),
            jcs(body.detail.as_ref().unwrap()),
            "{name}"
        );
        assert_eq!(
            body.typed_detail().unwrap(),
            None,
            "legacy detail API remains unchanged"
        );
    }
}

#[test]
fn legacy_detail_enum_still_supports_exhaustive_matches() {
    use cortexkit_role_agent_data::errors::{BusyDetail, Detail};
    let detail = Detail::Busy(BusyDetail::new(vec![]));
    let code = match detail {
        Detail::InvalidState(_) => RefusalCode::InvalidState,
        Detail::Busy(_) => RefusalCode::Busy,
        Detail::UnknownRef(_) => RefusalCode::UnknownRef,
        Detail::IncompatibleVersion(_) => RefusalCode::IncompatibleVersion,
        Detail::ExternalNotPresent(_) => RefusalCode::ExternalNotPresent,
    };
    assert_eq!(code, RefusalCode::Busy);
}

#[test]
fn refusal_bodies_tolerate_extra_members_but_details_are_strict() {
    let file = load("refusals.json");
    for case in cases(&file, "tolerated") {
        let body: ErrorBody = serde_json::from_value(case["body"].clone()).unwrap();
        assert_eq!(
            jcs(&serde_json::to_value(&body).unwrap()),
            jcs(&case["decodes_as"]),
            "{}",
            name(case)
        );
    }
    for case in cases(&file, "detail_refused") {
        let body: ErrorBody = serde_json::from_value(case["body"].clone()).unwrap();
        assert!(
            body.typed_detail().is_err(),
            "{}: detail decoded",
            name(case)
        );
    }
}

#[test]
fn idempotency_outcomes() {
    let file = load("idempotency.json");
    for case in cases(&file, "cases") {
        let name = name(case);
        let decode = |part: &str| {
            let request = &case[part];
            Request::decode(request["method"].as_str().unwrap(), &request["params"])
                .unwrap_or_else(|e| panic!("{name}: {part}: {e:?}"))
        };
        let expected = match case["outcome"].as_str().unwrap() {
            "replay" => Repeat::Replay,
            "id_reused" => Repeat::Reused,
            "new_key" => Repeat::NewKey,
            "exempt" => Repeat::Exempt,
            other => panic!("{name}: outcome {other}"),
        };
        assert_eq!(
            compare(&decode("first"), &decode("again")),
            expected,
            "{name}"
        );
    }
}

#[test]
fn step_machine_matches_every_transition() {
    let file = load("steps.json");
    let rows = cases(&file, "transitions");
    assert_eq!(
        rows.len(),
        Op::ALL.len() * Step::ALL.len(),
        "every (op, step) pair"
    );
    let mut seen = BTreeSet::new();
    for row in rows {
        let op = Op::ALL
            .into_iter()
            .find(|op| op.method() == format!("agent_data.{}", row["op"].as_str().unwrap()))
            .unwrap();
        let from: Step = serde_json::from_value(row["from"].clone()).unwrap();
        assert!(seen.insert((op.method(), from.as_str())), "{row}");
        let expected = if let Some(to) = row.get("to") {
            Ok(serde_json::from_value::<Step>(to.clone()).unwrap())
        } else {
            match row["refused"].as_str().unwrap() {
                "cut_unknown" => Err(StepRefusal::CutUnknown),
                "invalid_state" => Err(StepRefusal::InvalidState {
                    step: serde_json::from_value(row["step"].clone()).unwrap(),
                }),
                other => panic!("{row}: refused {other}"),
            }
        };
        assert_eq!(advance(op, from), expected, "{row}");
    }
}

/// Each `op-refusals.json` row plus its `digest-mismatch.json` additions equals
/// `allowed_refusals`, and each case's answer equals `may_refuse`.
#[test]
fn per_op_refusals_match_every_case() {
    let file = load("op-refusals.json");
    let amendments = load("digest-mismatch.json");
    let additions = cases(&amendments, "op_additions");
    assert_eq!(additions.len(), 2);
    let mut amended = BTreeSet::new();
    let rows = cases(&file, "ops");
    assert_eq!(rows.len(), Op::ALL.len());
    for (row, op) in rows.iter().zip(Op::ALL) {
        assert_eq!(
            format!("agent_data.{}", row["op"].as_str().unwrap()),
            op.method()
        );
        let mut codes: Vec<RefusalCode> = serde_json::from_value(row["codes"].clone()).unwrap();
        for addition in additions
            .iter()
            .filter(|addition| addition["op"] == row["op"])
        {
            assert!(
                amended.insert(op.method()),
                "duplicate amendment for {op:?}"
            );
            let extra: Vec<RefusalCode> =
                serde_json::from_value(addition["codes"].clone()).unwrap();
            assert!(!extra.is_empty());
            codes.extend(extra);
        }
        assert_eq!(codes, allowed_refusals(op), "{op:?}");
    }
    assert_eq!(
        amended.len(),
        additions.len(),
        "every additive row was checked"
    );
    for case in cases(&file, "cases") {
        let name = name(case);
        let op = Op::from_method(&format!("agent_data.{}", case["op"].as_str().unwrap())).unwrap();
        let code: RefusalCode = serde_json::from_value(case["code"].clone()).unwrap();
        assert_eq!(
            may_refuse(op, &code),
            case["allowed"].as_bool().unwrap(),
            "{name}"
        );
    }
}

/// The one-ref-set-per-cutId rule: `check_ref_set` gives each case's
/// outcome, and a refusal's body round-trips as an `invalid_state` whose
/// typed detail is the one `check_ref_set` returned.
#[test]
fn ref_set_rule_matches_every_case() {
    let file = load("ref-sets.json");
    let mut refusals = 0;
    for case in cases(&file, "cases") {
        let name = name(case);
        let op = Op::from_method(&format!("agent_data.{}", case["op"].as_str().unwrap())).unwrap();
        let step: Step = serde_json::from_value(case["step"].clone()).unwrap();
        let started: Option<Refs> = serde_json::from_value(case["started_with"].clone()).unwrap();
        let refs: Refs = serde_json::from_value(case["refs"].clone()).unwrap();
        let outcome = check_ref_set(op, step, started.as_ref(), &refs);
        if case["outcome"] == "ok" {
            assert_eq!(outcome, Ok(()), "{name}");
            continue;
        }
        refusals += 1;
        let detail = outcome.expect_err(name);
        let body = ErrorBody::with_typed_detail(
            cortexkit_role_agent_data::errors::Detail::InvalidState(detail),
            "ref set differs",
        );
        assert_eq!(body.code, RefusalCode::InvalidState, "{name}");
        let mut expected = case["outcome"].clone();
        expected["message"] = Value::from("ref set differs");
        let decoded: ErrorBody = round_trip(name, &expected);
        assert_eq!(decoded, body, "{name}");
    }
    assert!(refusals > 0);
}

/// Every `sessionRef` the vectors use is a value derived in
/// `session-refs.json`, and each derivation is what
/// `cortexkit_role_llm_runner::session_ref` returns for its inputs.
#[test]
fn session_refs_are_the_llm_runner_function() {
    let file = load("session-refs.json");
    let mut derived = BTreeSet::new();
    for case in cases(&file, "cases") {
        let value =
            cortexkit_role_llm_runner::session_ref(&cortexkit_role_llm_runner::SessionIdentity {
                runner: case["runner"].as_str().unwrap(),
                harness: case["harness"].as_str().unwrap(),
                session: case["session"].as_str().unwrap(),
            });
        assert_eq!(value, case["sessionRef"], "{case}");
        assert!(is_session_ref(&value));
        derived.insert(value);
    }
    fn collect(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::Object(map) => {
                for (key, value) in map {
                    if key == "sessionRef" {
                        out.insert(value.as_str().unwrap().to_owned());
                    } else {
                        collect(value, out);
                    }
                }
            }
            Value::Array(items) => items.iter().for_each(|item| collect(item, out)),
            _ => {}
        }
    }
    let mut used = BTreeSet::new();
    collect(&load("replies.json")["replies"], &mut used);
    assert!(!used.is_empty());
    assert!(used.is_subset(&derived), "{used:?} not all in {derived:?}");
}

/// Every store a vector declares uses one of the closed `sync_class`
/// spellings, so the vectors can't drift from the documented set.
#[test]
fn every_vector_store_uses_a_listed_sync_class() {
    let mut seen = 0;
    for file in ["replies.json", "requests.json"] {
        let mut stack = vec![load(file)];
        while let Some(value) = stack.pop() {
            match value {
                Value::Object(map) => {
                    if let Some(Value::String(class)) = map.get("sync_class") {
                        assert!(
                            cortexkit_role_agent_data::describe::sync_class::ALL
                                .contains(&class.as_str()),
                            "{file}: sync_class {class:?} is not in the closed set"
                        );
                        seen += 1;
                    }
                    stack.extend(map.into_values());
                }
                Value::Array(items) => stack.extend(items),
                _ => {}
            }
        }
    }
    assert!(
        seen > 0,
        "no vector declares a sync_class, so this test checks nothing"
    );
}
