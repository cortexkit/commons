//! Tests over `test-vectors/compaction-provider-v1/async.json`. Its digests
//! were computed by a separate implementation, so these tests compare this
//! crate against a second one rather than against itself.

use std::collections::BTreeMap;

use serde_json::Value;

use super::{
    ceiling::{CeilingDecision, CeilingPolicy, WaitEnd},
    content_digest,
    notice::{self, AdvanceNotice, Event, LastUsage, MarkerOutcome, ViewReady},
    ride::{RideAnswer, RideCause, RideRequest},
    view::{
        dispose_fetched, CompactionView, PartEdit, TranscriptMessage, ViewCut, ViewGetAnswer,
        ViewGetRequest, ViewRange,
    },
};
use crate::{
    describe::{check_describe, DescribeProblem, RoleDescribe},
    setup::{accept_mode, session_mode, CompactionMode, SetupAnswer, SetupRequest},
    status::MessageRef,
    vectors, OpRequest,
};

fn file() -> Value {
    vectors::load("async.json")
}

fn name(case: &Value) -> &str {
    case["name"].as_str().unwrap()
}

fn headers(json: &Value) -> BTreeMap<String, String> {
    serde_json::from_value(json.clone()).unwrap()
}

fn str_of<'a>(file: &'a Value, key: &str) -> &'a str {
    file["session"][key].as_str().unwrap()
}

#[test]
fn async_fixture_matches_sha256sum() {
    let sums = std::fs::read_to_string(format!("{}/ASYNC_SHA256SUMS", vectors::dir())).unwrap();
    let lines: Vec<_> = sums.lines().collect();
    assert_eq!(lines.len(), 1);
    let (digest, file_name) = lines[0].split_once("  ").unwrap();
    assert_eq!(file_name, "async.json");
    let bytes = std::fs::read(format!("{}/{file_name}", vectors::dir())).unwrap();
    assert_eq!(super::sha256_hex(&bytes), digest);
}

#[test]
fn session_ref_is_the_runner_roles_reference() {
    let file = file();
    assert_eq!(
        cortexkit_role_llm_runner::session_ref::session_ref(
            str_of(&file, "harness"),
            str_of(&file, "project_root"),
            str_of(&file, "session"),
        ),
        str_of(&file, "session_ref")
    );
}

#[test]
fn describe_declares_the_async_group_with_its_ops() {
    let file = file();
    for case in vectors::cases(&file["describe"], "accepted") {
        let name = name(case);
        let describe = vectors::round_trip::<RoleDescribe>(name, &case["answer"]);
        let checked = check_describe(&case["answer"]).unwrap_or_else(|p| panic!("{name}: {p:?}"));
        assert_eq!(
            checked.declares_async(),
            case["declares_async"].as_bool().unwrap()
        );
        if describe.declares_async() {
            let mut bare = case["answer"].clone();
            bare["capabilities"] = Value::Array(Vec::new());
            let built = serde_json::from_value::<RoleDescribe>(bare)
                .unwrap()
                .with_async()
                .with_async();
            assert_eq!(built, describe, "{name}: with_async adds the group once");
        }
    }
    for case in vectors::cases(&file["describe"], "refused") {
        let name = name(case);
        let problems = check_describe(&case["answer"]).expect_err(name);
        let op = case["op"].as_str().unwrap();
        assert!(
            problems.contains(&DescribeProblem::MissingOp(
                super::GROUP_OPS.iter().find(|o| **o == op).unwrap()
            )),
            "{name}: {problems:?}"
        );
        assert!(problems
            .iter()
            .all(|p| p.name() == case["problem"].as_str().unwrap()));
    }
}

#[test]
fn notices_round_trip_with_independent_digests() {
    let file = file();
    let account = str_of(&file, "account");
    let runner = str_of(&file, "runner_module_id");
    let transcript = &file["transcript"]["messages"];
    for case in vectors::cases(&file, "notices") {
        let name = name(case);
        let map = headers(&case["headers"]);
        let notice = AdvanceNotice::from_headers(&map).unwrap_or_else(|p| panic!("{name}: {p:?}"));
        assert_eq!(notice.to_headers(), map, "{name}");
        assert_eq!(notice.digest(), case["digest"].as_str().unwrap(), "{name}");
        assert_eq!(
            notice.message_id(),
            case["message_id"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(
            notice::event_subject(account, runner, notice::ADVANCED_EVENT),
            case["subject"].as_str().unwrap()
        );
        let message = transcript
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["ordinal"].as_u64() == Some(notice.through_ordinal))
            .unwrap();
        assert_eq!(
            content_digest(&message["message"]),
            notice.through_digest,
            "{name}"
        );
        assert!(notice::header_bytes(&map) <= notice::MAX_NOTICE_HEADER_BYTES);
    }
}

#[test]
fn notice_builders_match_the_vector() {
    let file = file();
    let case = &vectors::cases(&file, "notices")[1];
    let decoded = AdvanceNotice::from_headers(&headers(&case["headers"])).unwrap();
    let built = AdvanceNotice::new(
        str_of(&file, "session_ref"),
        5,
        4,
        decoded.through_digest.clone(),
        64000,
        "anthropic/claude-sonnet-4-5",
    )
    .with_window(200000)
    .with_last_usage(LastUsage::new(61000, 58000, 2000, 850))
    .with_applied_view_version(3)
    .with_events(vec![Event::MarkerOutcome {
        outcome: MarkerOutcome::Applied,
        cut_id: "cut-1".into(),
    }]);
    assert_eq!(built, decoded);
    assert_eq!(built.check(), Ok(()));
}

#[test]
fn notices_accepted_ignore_what_they_do_not_know() {
    let file = file();
    for case in vectors::cases(&file, "notices_accepted") {
        let name = name(case);
        let map = headers(&case["headers"]);
        let notice = AdvanceNotice::from_headers(&map).unwrap_or_else(|p| panic!("{name}: {p:?}"));
        let tokens: Vec<String> = notice.events.iter().map(Event::token).collect();
        let expected: Vec<String> = serde_json::from_value(case["events"].clone()).unwrap();
        assert_eq!(tokens, expected, "{name}");
        assert_eq!(
            notice::notice_digest(&map),
            case["digest"].as_str().unwrap(),
            "{name}"
        );
        assert_eq!(notice.digest(), case["digest"].as_str().unwrap(), "{name}");
    }
    let unknown = Event::parse("cache_warmed:5m").unwrap();
    assert_eq!(unknown, Event::Unknown("cache_warmed:5m".into()));
}

#[test]
fn notices_refused_by_name() {
    let file = file();
    for case in vectors::cases(&file, "notices_refused") {
        let name = name(case);
        let problem = AdvanceNotice::from_headers(&headers(&case["headers"]))
            .expect_err(name)
            .name();
        assert_eq!(problem, case["problem"].as_str().unwrap(), "{name}");
    }
}

#[test]
fn the_largest_well_formed_notice_fits_the_header_bound() {
    let max = u64::MAX;
    let events: Vec<Event> = (0..9)
        .map(|i| Event::Unknown(format!("e{i}_{}", "x".repeat(108))))
        .collect();
    let notice = AdvanceNotice::new(
        "0".repeat(32),
        max,
        max,
        "a".repeat(64),
        max,
        "m".repeat(256),
    )
    .with_window(max)
    .with_last_usage(LastUsage::new(max, max, max, max))
    .with_applied_view_version(max)
    .with_events(events);
    let encoded = notice.to_headers();
    assert!(encoded[notice::headers::EVENTS].len() <= notice::MAX_HEADER_VALUE_BYTES);
    assert_eq!(notice.check(), Ok(()));
    // Every value at its bound: `events` at 1,024 bytes, the rest at their
    // widest encodings.
    let names: usize = notice::headers::NOTICE.iter().map(|n| n.len()).sum();
    let widest = names + 1024 + 4 * 20 + 3 + 256 + 32 + 64 + 5 * 20;
    assert_eq!(widest, 1659);
    assert!(widest <= notice::MAX_NOTICE_HEADER_BYTES);
    assert!(notice::header_bytes(&encoded) <= widest);
}

#[test]
fn view_ready_vectors() {
    let file = file();
    for case in vectors::cases(&file, "view_ready") {
        let name = name(case);
        let map = headers(&case["headers"]);
        let ready = ViewReady::from_headers(&map).unwrap_or_else(|p| panic!("{name}: {p:?}"));
        assert_eq!(ready.to_headers(), map, "{name}");
        assert_eq!(ready.message_id(), case["message_id"].as_str().unwrap());
        assert_eq!(ready.digest, case["bus_digest"].as_str().unwrap());
        assert_eq!(
            notice::event_subject(
                str_of(&file, "account"),
                str_of(&file, "provider_module_id"),
                notice::VIEW_READY_EVENT
            ),
            case["subject"].as_str().unwrap()
        );
        let view: CompactionView =
            serde_json::from_value(vectors::cases(&file, "views")[1]["view"].clone()).unwrap();
        assert_eq!(ViewReady::for_view(&view), ready);
        assert_eq!(
            ViewGetRequest::for_ready(&ready),
            ViewGetRequest::new(ready.session_ref.clone(), 3)
        );
    }
    for case in vectors::cases(&file, "view_ready_refused") {
        let name = name(case);
        let problem = ViewReady::from_headers(&headers(&case["headers"]))
            .expect_err(name)
            .name();
        assert_eq!(problem, case["problem"].as_str().unwrap(), "{name}");
    }
}

fn transcript_of(rows: &Value) -> Vec<TranscriptMessage> {
    serde_json::from_value(rows.clone()).unwrap()
}

fn transcript(file: &Value) -> Vec<TranscriptMessage> {
    transcript_of(&file["transcript"]["messages"])
}

#[test]
fn views_check_and_assemble_as_recorded() {
    let file = file();
    let transcript = transcript(&file);
    for case in vectors::cases(&file, "views") {
        let name = name(case);
        let view = vectors::round_trip::<CompactionView>(name, &case["view"]);
        assert_eq!(view.check(), Ok(()), "{name}");
        assert_eq!(view.compute_digest(), view.digest, "{name}");
        let assembled = view.assemble(&transcript).unwrap();
        assert_eq!(Value::Array(assembled), case["assembled"], "{name}");
    }
}

#[test]
fn view_builders_compute_the_vector_digest() {
    let file = file();
    let case = &vectors::cases(&file, "views")[1];
    let decoded: CompactionView = serde_json::from_value(case["view"].clone()).unwrap();
    let (head, edits) = match (&decoded.ranges[0], &decoded.ranges[1]) {
        (ViewRange::Replace { bytes, .. }, ViewRange::Keep { edits, .. }) => (bytes, edits),
        other => panic!("unexpected ranges {other:?}"),
    };
    let placeholder = edits[0].bytes.clone().unwrap();
    let built = CompactionView::new(
        str_of(&file, "session_ref"),
        3,
        "lin_a",
        MessageRef {
            ordinal: 4,
            mid: "m4".into(),
        },
        vec![
            ViewRange::replace(0, 2, head.clone()),
            ViewRange::keep_with_edits(
                2,
                5,
                vec![
                    PartEdit::replace(3, "m3", "prt_3a", placeholder),
                    PartEdit::drop(3, "m3", "prt_3b"),
                ],
            ),
        ],
    )
    .with_cut(ViewCut::new("cut-1", 1));
    assert_eq!(built, decoded);
}

#[test]
fn views_refused_by_name() {
    let file = file();
    for case in vectors::cases(&file, "views_refused") {
        let name = name(case);
        let view: CompactionView = serde_json::from_value(case["view"].clone())
            .unwrap_or_else(|e| panic!("{name}: does not decode: {e}"));
        let problem = view.check().expect_err(name);
        assert_eq!(problem.name(), case["problem"].as_str().unwrap(), "{name}");
    }
    for case in vectors::cases(&file, "views_undecodable") {
        vectors::refused::<CompactionView>(name(case), &case["view"]);
    }
}

#[test]
fn part_edits_refused_by_name() {
    let file = file();
    let cases = vectors::cases(&file, "edits_refused");
    assert!(!cases.is_empty());
    for case in cases {
        let name = name(case);
        let view = vectors::round_trip::<CompactionView>(name, &case["view"]);
        let problem = view.check().expect_err(name);
        assert_eq!(problem.name(), case["problem"].as_str().unwrap(), "{name}");
    }
}

/// The dispositions with an outcome in `outcomes`, each checked against
/// [`dispose_fetched`]. Returns how many were checked.
fn check_dispositions(outcomes: &[&str]) -> usize {
    let file = file();
    let mut checked = 0;
    for case in vectors::cases(&file, "dispositions") {
        let outcome = case["outcome"].as_str().unwrap();
        if !outcomes.contains(&outcome) {
            continue;
        }
        let name = name(case);
        let view: CompactionView = serde_json::from_value(case["view"].clone()).unwrap();
        let ready = ViewReady::from_headers(&headers(&case["ready"])).unwrap();
        let held = transcript_of(&case["transcript"]);
        let disposition = dispose_fetched(
            &view,
            &ready,
            case["current_version"].as_u64(),
            case["lineage_id"].as_str().unwrap(),
            |ordinal| held.iter().find(|m| m.ordinal == ordinal).cloned(),
        );
        assert_eq!(disposition.name(), outcome, "{name}");
        checked += 1;
    }
    checked
}

const APPLY_OR_STALE: &[&str] = &["apply", "stale"];
const DISCARD: &[&str] = &["discard"];
const UNKNOWN_PART: &[&str] = &["refused:unknown_part"];
const DIGEST: &[&str] = &["rejected:digest_mismatch"];
const OTHER_REJECTIONS: &[&str] = &[
    "rejected:not_requested",
    "rejected:gap",
    "rejected:edit_order",
];

#[test]
fn every_disposition_vector_is_checked_by_one_test() {
    let file = file();
    for case in vectors::cases(&file, "dispositions") {
        let outcome = case["outcome"].as_str().unwrap();
        let groups = [
            APPLY_OR_STALE,
            DISCARD,
            UNKNOWN_PART,
            DIGEST,
            OTHER_REJECTIONS,
        ];
        let owners = groups.iter().filter(|g| g.contains(&outcome)).count();
        assert_eq!(owners, 1, "{}: outcome {outcome}", name(case));
    }
}

#[test]
fn fetched_view_applies_or_is_stale() {
    assert_eq!(check_dispositions(APPLY_OR_STALE), 3);
}

#[test]
fn view_whose_coverage_is_no_longer_a_prefix_is_discarded() {
    assert_eq!(check_dispositions(DISCARD), 4);
}

#[test]
fn view_editing_an_unknown_part_is_refused() {
    assert_eq!(check_dispositions(UNKNOWN_PART), 1);
}

#[test]
fn fetched_view_failing_its_digest_is_rejected() {
    assert_eq!(check_dispositions(DIGEST), 2);
}

#[test]
fn fetched_view_not_requested_or_malformed_is_rejected() {
    assert_eq!(check_dispositions(OTHER_REJECTIONS), 3);
}

#[test]
fn view_get_vectors() {
    let file = file();
    let view_get = &file["view_get"];
    for case in vectors::cases(view_get, "requests") {
        let request =
            vectors::round_trip::<OpRequest<ViewGetRequest>>(name(case), &case["request"]);
        assert_eq!(request.method, crate::ops::VIEW_GET);
    }
    for case in vectors::cases(view_get, "answers") {
        let answer = vectors::round_trip::<ViewGetAnswer>(name(case), &case["answer"]);
        match answer {
            ViewGetAnswer::View { view } => assert_eq!(view.check(), Ok(())),
            ViewGetAnswer::Refuse { code, .. } => assert!(
                [
                    super::view::view_get_codes::UNKNOWN_SESSION,
                    super::view::view_get_codes::UNKNOWN_VERSION
                ]
                .contains(&code.as_str()),
                "{code}"
            ),
        }
    }
    for case in vectors::cases(view_get, "undecodable_answers") {
        vectors::refused::<ViewGetAnswer>(name(case), &case["answer"]);
    }
}

#[test]
fn ride_vectors() {
    let file = file();
    let ride = &file["ride"];
    let mut causes = Vec::new();
    for case in vectors::cases(ride, "requests") {
        let request = vectors::round_trip::<OpRequest<RideRequest>>(name(case), &case["request"]);
        assert_eq!(request.method, crate::ops::COMPACTION_RIDE);
        assert_eq!(request.params.check(), Ok(()), "{}", name(case));
        causes.push(request.params.cause);
    }
    assert_eq!(
        causes,
        [
            RideCause::TtlExpired,
            RideCause::ModelChange,
            RideCause::SurfaceChange,
            RideCause::Ceiling,
            RideCause::Unknown("provider_switch".into()),
        ]
    );
    for case in vectors::cases(ride, "requests_refused") {
        let request: RideRequest = serde_json::from_value(case["params"].clone()).unwrap();
        assert_eq!(
            request.check().expect_err(name(case)).name(),
            case["problem"].as_str().unwrap()
        );
    }
    let answers: Vec<RideAnswer> = vectors::cases(ride, "answers")
        .iter()
        .map(|case| vectors::round_trip::<RideAnswer>(name(case), &case["answer"]))
        .collect();
    assert!(matches!(&answers[0], RideAnswer::View { view } if view.check().is_ok()));
    assert_eq!(answers[1], RideAnswer::nothing());
    for case in vectors::cases(ride, "undecodable_answers") {
        vectors::refused::<RideAnswer>(name(case), &case["answer"]);
    }
}

#[test]
fn ceiling_vectors() {
    let file = file();
    let ceiling = &file["ceiling"];
    let answer = vectors::round_trip::<SetupAnswer>("setup with ceiling", &ceiling["setup_answer"]);
    match answer {
        SetupAnswer::Ready { ceiling, .. } => assert_eq!(
            ceiling,
            Some(CeilingPolicy::new(0.9, 30000).with_on_overflow())
        ),
        other => panic!("expected ready, got {other:?}"),
    }
    for case in vectors::cases(ceiling, "policies") {
        let policy = vectors::round_trip::<CeilingPolicy>(name(case), &case["policy"]);
        assert_eq!(policy.check(), Ok(()), "{}", name(case));
    }
    for case in vectors::cases(ceiling, "policies_refused") {
        let policy: CeilingPolicy = serde_json::from_value(case["policy"].clone()).unwrap();
        assert_eq!(
            policy.check().expect_err(name(case)).name(),
            case["problem"].as_str().unwrap()
        );
    }
    let policy = CeilingPolicy::new(0.9, 30000);
    for case in vectors::cases(ceiling, "reached") {
        assert_eq!(
            policy.reached(
                case["size"].as_u64().unwrap(),
                case["window"].as_u64().unwrap()
            ),
            case["reached"].as_bool().unwrap(),
            "{case}"
        );
    }
    for case in vectors::cases(ceiling, "decisions") {
        let end = match case["end"].as_str().unwrap() {
            "view_applied" => WaitEnd::ViewApplied,
            "timed_out" => WaitEnd::TimedOut,
            "provider_unavailable" => WaitEnd::ProviderUnavailable,
            other => panic!("unknown end {other}"),
        };
        let decision = policy.decide(
            case["size"].as_u64().unwrap(),
            case["window"].as_u64().unwrap(),
            case["overflowed"].as_bool().unwrap(),
            end,
        );
        let spelled = match decision {
            CeilingDecision::Send => "send".to_owned(),
            CeilingDecision::Refuse { cause } => format!("refuse:{cause}"),
        };
        assert_eq!(
            spelled,
            case["decision"].as_str().unwrap(),
            "{}",
            name(case)
        );
    }
}

#[test]
fn ceiling_absent_keeps_existing_setup_bytes() {
    let setup = vectors::load("setup.json");
    for case in vectors::cases(&setup, "answers") {
        let answer: SetupAnswer = serde_json::from_value(case["answer"].clone()).unwrap();
        if let SetupAnswer::Ready { ceiling, .. } = answer {
            assert_eq!(ceiling, None, "{}", name(case));
        }
    }
}

fn mode_of(value: &Value) -> Option<CompactionMode> {
    (!value.is_null()).then(|| serde_json::from_value(value.clone()).unwrap())
}

#[test]
fn setup_mode_vectors_decode_strictly() {
    let file = file();
    let mode = &file["mode"];
    let requested: Vec<Option<CompactionMode>> = vectors::cases(mode, "requests")
        .iter()
        .map(|case| {
            vectors::round_trip::<SetupRequest>(name(case), &case["request"]).compaction_mode
        })
        .collect();
    assert_eq!(
        requested,
        [
            None,
            Some(CompactionMode::Sync),
            Some(CompactionMode::Async)
        ]
    );
    for case in vectors::cases(mode, "undecodable_requests") {
        vectors::refused::<SetupRequest>(name(case), &case["request"]);
    }
    for case in vectors::cases(mode, "answers") {
        let answer = vectors::round_trip::<SetupAnswer>(name(case), &case["answer"]);
        if let Some(retryable) = case.get("retryable") {
            assert_eq!(answer.retryable(), retryable.as_bool(), "{}", name(case));
        }
    }
    for case in vectors::cases(mode, "undecodable_answers") {
        vectors::refused::<SetupAnswer>(name(case), &case["answer"]);
    }
}

#[test]
fn session_is_async_only_when_asked_and_echoed() {
    let file = file();
    let answers = vectors::cases(&file["mode"], "answers");
    let ready: SetupAnswer = serde_json::from_value(answers[1]["answer"].clone()).unwrap();
    let refused: SetupAnswer = serde_json::from_value(answers[2]["answer"].clone()).unwrap();
    for case in vectors::cases(&file["mode"], "session_mode") {
        let answer = match case["echo"].as_str() {
            Some("refuse") => refused.clone(),
            echo => {
                let mut answer = ready.clone();
                if let SetupAnswer::Ready {
                    compaction_mode, ..
                } = &mut answer
                {
                    *compaction_mode = echo.map(|_| mode_of(&case["echo"]).unwrap());
                }
                answer
            }
        };
        let expected: CompactionMode = serde_json::from_value(case["mode"].clone()).unwrap();
        assert_eq!(
            session_mode(mode_of(&case["requested"]), &answer),
            expected,
            "{case}"
        );
    }
}

#[test]
fn provider_accepts_async_only_for_a_runner_declaring_compaction_async() {
    let file = file();
    for case in vectors::cases(&file["mode"], "acceptance") {
        let capabilities: Vec<String> =
            serde_json::from_value(case["runner_capabilities"].clone()).unwrap();
        let outcome = match accept_mode(
            mode_of(&case["requested"]),
            case["provider_declares_async"].as_bool().unwrap(),
            &capabilities,
        ) {
            Ok(Some(CompactionMode::Async)) => "echo:async".to_owned(),
            Ok(Some(CompactionMode::Sync)) => "echo:sync".to_owned(),
            Ok(None) => "no_echo".to_owned(),
            Err(code) => format!("refuse:{}", code.as_str()),
        };
        assert_eq!(outcome, case["outcome"].as_str().unwrap(), "{}", name(case));
    }
    assert_eq!(super::RUNNER_GROUP, "compaction_async");
}
