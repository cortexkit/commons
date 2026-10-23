//! The role's vectors under `test-vectors/routing-v1/`, read through the
//! crate's `vectors` module (the `test-vectors` feature). Every request,
//! answer and refusal is decoded into the crate's types and re-encoded, and
//! must come back as the same JSON value: a member a type drops, renames or
//! invents shows up here. Request vectors also go through the crate's
//! validators, and refused vectors must be refused with exactly the code and
//! detail they record.

use std::fmt::Debug;

use cortexkit_role_routing::{
    account::{parse_account_for_request, AccountForAnswer, AccountForRequest},
    describe::{check_describe, RoleDescribe},
    errors::{
        AccountHoldDetail, FieldDetail, NoRouteDetail, PinUnavailableDetail, Refusal, RefusalCode,
        RoutingAmbiguousDetail, UnknownConsumerDetail, CONSUMER_CODES,
    },
    observe::{parse_account_observe_request, AccountObserveAnswer, AccountObserveRequest},
    report::{parse_report_request, ReportAnswer, ReportRequest},
    select::{parse_select_request, SelectAnswer, SelectRequest, REQUIREMENT_FIELDS},
    vectors, wire, REQUIRED_OPS,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

/// Every vector file this test reads. A test checks this list against
/// `vectors::FILES`, which `tests/vectors_feature.rs` checks against the
/// directory, so a new file cannot land without a test reading it.
const FILES: &[&str] = &[
    "open-sets.json",
    "refusals.json",
    "reply-envelope.json",
    "role-describe.json",
    "route-account-for.json",
    "route-account-observe.json",
    "route-report.json",
    "route-select.json",
];

fn load(name: &str) -> Value {
    assert!(FILES.contains(&name), "{name} is not listed in FILES");
    let text = vectors::file(name).unwrap_or_else(|| panic!("{name} is not in vectors::FILES"));
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn cases<'a>(file: &'a Value, key: &str) -> &'a Vec<Value> {
    file[key]
        .as_array()
        .unwrap_or_else(|| panic!("no {key} array"))
}

fn name(case: &Value) -> &str {
    case["name"].as_str().expect("every case has a name")
}

/// Decode `value` as `T`, check it re-encodes to `expected`, and return it.
fn round_trip<T: DeserializeOwned + Serialize>(case: &str, value: &Value, expected: &Value) -> T {
    let decoded: T =
        serde_json::from_value(value.clone()).unwrap_or_else(|e| panic!("{case}: {e}"));
    assert_eq!(
        &serde_json::to_value(&decoded).unwrap(),
        expected,
        "{case}: re-encoding differs"
    );
    decoded
}

/// Check a file's `requests`, `refused` and `answers` against a validator
/// and the request and answer types.
fn check_op<Req, Ans>(file: &str, parse: fn(&Value) -> Result<Req, Refusal>)
where
    Req: DeserializeOwned + Serialize + PartialEq + Debug,
    Ans: DeserializeOwned + Serialize,
{
    let file = load(file);
    for case in cases(&file, "requests") {
        let name = name(case);
        let params = &case["params"];
        let expected = case.get("encodes_as").unwrap_or(params);
        let parsed = parse(params).unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        assert_eq!(&serde_json::to_value(&parsed).unwrap(), expected, "{name}");
        let decoded: Req = round_trip(name, params, expected);
        assert_eq!(parsed, decoded, "{name}: the validator and serde disagree");
    }
    for case in cases(&file, "refused") {
        let name = name(case);
        let refusal = parse(&case["params"]).expect_err(name);
        assert_eq!(refusal.code.as_str(), case["refusal"]["code"], "{name}");
        assert_eq!(
            Value::Object(refusal.detail.clone()),
            case["refusal"]["detail"],
            "{name}"
        );
        assert!(!refusal.message.is_empty(), "{name}");
    }
    for case in cases(&file, "answers") {
        let _: Ans = round_trip(name(case), &case["answer"], &case["answer"]);
    }
}

#[test]
fn every_file_is_read() {
    let mut shipped: Vec<&str> = vectors::FILES.iter().map(|(name, _)| *name).collect();
    shipped.sort_unstable();
    assert_eq!(shipped, FILES);
}

#[test]
fn role_describe() {
    let file = load("role-describe.json");
    assert_eq!(file["request"], json!({}));
    let describe: RoleDescribe = round_trip("answer", &file["answer"], &file["answer"]);
    assert_eq!(check_describe(&describe), Ok(()));
    assert_eq!(describe.ops, REQUIRED_OPS);
    assert_eq!(describe.requirement_fields, REQUIREMENT_FIELDS);
    assert!(describe.consumer("codemode").unwrap().priced_only);
    assert!(describe.consumer("flow").unwrap().priced_only);
}

#[test]
fn route_select() {
    check_op::<SelectRequest, SelectAnswer>("route-select.json", parse_select_request);
}

#[test]
fn route_report() {
    check_op::<ReportRequest, ReportAnswer>("route-report.json", parse_report_request);
    let file = load("route-report.json");
    for case in cases(&file, "bodies") {
        let first = parse_report_request(&case["first"]).unwrap();
        let second = parse_report_request(&case["second"]).unwrap();
        assert_eq!(first == second, case["same"], "{}", name(case));
    }
}

#[test]
fn route_account_for() {
    check_op::<AccountForRequest, AccountForAnswer>(
        "route-account-for.json",
        parse_account_for_request,
    );
    let file = load("route-account-for.json");
    for case in cases(&file, "bodies") {
        let first = parse_account_for_request(&case["first"]).unwrap();
        let second = parse_account_for_request(&case["second"]).unwrap();
        assert_eq!(first.same_body(&second), case["same"], "{}", name(case));
    }
}

#[test]
fn route_account_observe() {
    check_op::<AccountObserveRequest, AccountObserveAnswer>(
        "route-account-observe.json",
        parse_account_observe_request,
    );
    let file = load("route-account-observe.json");
    let answers = cases(&file, "answers");
    let first: AccountObserveAnswer = serde_json::from_value(answers[0]["answer"].clone()).unwrap();
    let again: AccountObserveAnswer = serde_json::from_value(answers[1]["answer"].clone()).unwrap();
    assert!(!first.observation.duplicate);
    assert!(again.observation.duplicate);
    assert_eq!(first.observation.id, again.observation.id);
    assert!(again.observation.effects.is_empty());
}

#[test]
fn every_refusal_code_has_a_vector() {
    let file = load("refusals.json");
    let mut seen = Vec::new();
    for case in cases(&file, "refusals") {
        let name = name(case);
        let body = &case["body"];
        let keys: Vec<&String> = body.as_object().unwrap().keys().collect();
        assert_eq!(
            keys.len(),
            3,
            "{name}: a refusal body is code, message, detail"
        );
        let refusal: Refusal = round_trip(name, body, body);
        // The same bytes read through the wire helper.
        let read = wire::read_refusal(&serde_json::to_vec(body).unwrap()).unwrap();
        assert_eq!(read, refusal, "{name}");
        let expected_wait = case["wait_until_ms"].as_u64();
        assert_eq!(refusal.wait_until_ms(), expected_wait, "{name}");
        let ops: Vec<&str> = case["ops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|op| op.as_str().unwrap())
            .collect();
        for op in &ops {
            assert!(REQUIRED_OPS.contains(op), "{name}: {op}");
        }
        let consumer_side = CONSUMER_CODES.contains(&refusal.code.as_str());
        assert_eq!(
            ops.is_empty(),
            consumer_side || !refusal.code.is_known(),
            "{name}: only consumer-side and unknown codes name no op"
        );
        let typed = match refusal.code {
            RefusalCode::InvalidRequest | RefusalCode::InvalidRequirements => {
                refusal.detail_as::<FieldDetail>().is_some()
            }
            RefusalCode::UnknownConsumer => refusal.detail_as::<UnknownConsumerDetail>().is_some(),
            RefusalCode::PinUnavailable => refusal.detail_as::<PinUnavailableDetail>().is_some(),
            RefusalCode::NoRoute => refusal.detail_as::<NoRouteDetail>().is_some(),
            RefusalCode::AccountHold => refusal.detail_as::<AccountHoldDetail>().is_some(),
            RefusalCode::RoutingAmbiguous => {
                refusal.detail_as::<RoutingAmbiguousDetail>().is_some()
            }
            _ => true,
        };
        assert!(
            typed,
            "{name}: the detail decodes as the code's detail type"
        );
        seen.push(refusal.code.as_str().to_owned());
    }
    for code in RefusalCode::KNOWN {
        assert!(seen.iter().any(|seen| seen == code), "no vector for {code}");
    }
    assert!(
        seen.iter().any(|code| !RefusalCode::parse(code).is_known()),
        "no vector for an unknown code"
    );
}

#[test]
fn unknown_open_set_values_decode_and_are_kept() {
    let file = load("open-sets.json");
    let mut types = Vec::new();
    for case in cases(&file, "cases") {
        let name = name(case);
        let value = &case["value"];
        let pointer = case["at"].as_str().unwrap();
        let unknown = value
            .pointer(pointer)
            .and_then(Value::as_str)
            .unwrap_or_else(|| panic!("{name}: nothing at {pointer}"));
        let kind = case["type"].as_str().unwrap();
        let (known, kept) = match kind {
            "Refusal" => {
                let decoded: Refusal = round_trip(name, value, value);
                (decoded.code.is_known(), decoded.code.as_str().to_owned())
            }
            "ReportRequest" => {
                let decoded: ReportRequest = round_trip(name, value, value);
                assert_eq!(parse_report_request(value).unwrap(), decoded, "{name}");
                (
                    decoded.outcome.is_known(),
                    decoded.outcome.as_str().to_owned(),
                )
            }
            "PinUnavailableDetail" => {
                let decoded: PinUnavailableDetail = round_trip(name, value, value);
                (
                    decoded.reason.is_known(),
                    decoded.reason.as_str().to_owned(),
                )
            }
            "NoRouteDetail" => {
                let decoded: NoRouteDetail = round_trip(name, value, value);
                (
                    decoded.reason.is_known(),
                    decoded.reason.as_str().to_owned(),
                )
            }
            "AccountObserveRequest" => {
                let decoded: AccountObserveRequest = round_trip(name, value, value);
                assert_eq!(
                    parse_account_observe_request(value).unwrap(),
                    decoded,
                    "{name}"
                );
                (decoded.kind.is_known(), decoded.kind.as_str().to_owned())
            }
            "AccountForAnswer" => {
                let decoded: AccountForAnswer = round_trip(name, value, value);
                (
                    decoded.reason.code.is_known(),
                    decoded.reason.code.as_str().to_owned(),
                )
            }
            other => panic!("{name}: no type {other}"),
        };
        assert!(!known, "{name}: {unknown} should be unknown to this crate");
        assert_eq!(kept, unknown, "{name}");
        types.push(kind);
    }
    for kind in [
        "Refusal",
        "ReportRequest",
        "PinUnavailableDetail",
        "NoRouteDetail",
        "AccountObserveRequest",
        "AccountForAnswer",
    ] {
        assert!(types.contains(&kind), "no unknown-value vector for {kind}");
    }
}

/// Read `body` as the answer type `answer` names, as a consumer of that op
/// would, and return the decoded answer as JSON.
fn read_named(answer: &str, body: &[u8]) -> Result<Value, wire::ReplyShapeError> {
    fn as_value<T: Serialize>(answer: T) -> Value {
        serde_json::to_value(answer).unwrap()
    }
    match answer {
        "RoleDescribe" => wire::read_reply::<RoleDescribe>(body).map(as_value),
        "SelectAnswer" => wire::read_reply::<SelectAnswer>(body).map(as_value),
        "ReportAnswer" => wire::read_reply::<ReportAnswer>(body).map(as_value),
        "AccountForAnswer" => wire::read_reply::<AccountForAnswer>(body).map(as_value),
        "AccountObserveAnswer" => wire::read_reply::<AccountObserveAnswer>(body).map(as_value),
        other => panic!("no answer type {other}"),
    }
}

#[test]
fn reply_envelope() {
    let file = load("reply-envelope.json");
    let mut refused = Vec::new();
    for case in cases(&file, "replies") {
        let name = name(case);
        let body = serde_json::to_vec(&case["body"]).unwrap();
        let read = read_named(case["answer"].as_str().unwrap(), &body);
        match case.get("refused") {
            None => {
                let answer = read.unwrap_or_else(|e| panic!("{name}: {e}"));
                assert_eq!(answer, case["body"][wire::RESULT_KEY], "{name}");
                assert_eq!(
                    serde_json::from_slice::<Value>(&wire::reply_body(&answer).unwrap()).unwrap(),
                    case["body"],
                    "{name}"
                );
            }
            Some(problem) => {
                let error = read.expect_err(name);
                assert_eq!(error.name(), problem, "{name}");
                refused.push(error.name());
            }
        }
    }
    for problem in ["bare_answer", "extra_keys", "bad_answer"] {
        assert!(refused.contains(&problem), "no vector refuses {problem}");
    }
}
