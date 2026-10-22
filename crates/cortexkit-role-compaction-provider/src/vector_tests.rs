//! Tests over this crate's `test-vectors/compaction-provider-v1/async.json`. Its digests
//! were computed by a separate implementation, so these tests compare this
//! crate against a second one rather than against itself.

use std::collections::BTreeMap;

use serde_json::Value;

use crate::{
    ceiling::{CeilingDecision, CeilingPolicy, WaitEnd},
    digest::{content_digest, sha256_hex},
    lineage::MessageRef,
    notice::{
        self, descent_outcome, AdvanceNotice, DescentOutcome, Event, LastUsage, MarkerOutcome,
        NoticeDescent, ViewReady,
    },
    ride::{Overflow, RideAnswer, RideCause, RideRequest},
    setup::{SetupAnswer, SetupRequest},
    vectors,
    view::{
        self, dispose_fetched, CompactionView, PartEdit, TranscriptMessage, ViewCut, ViewGetAnswer,
        ViewGetRequest, ViewRange,
    },
    OpRequest,
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
    assert_eq!(sha256_hex(&bytes), digest);
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
        "lin_a",
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
        "l".repeat(256),
        max,
        "a".repeat(64),
        max,
        "m".repeat(256),
    )
    .with_window(max)
    .with_last_usage(LastUsage::new(max, max, max, max))
    .with_applied_view_version(max)
    .with_descends_from(NoticeDescent::new("p".repeat(256), max, "b".repeat(64)))
    .with_events(events);
    let encoded = notice.to_headers();
    assert!(encoded[notice::headers::EVENTS].len() <= notice::MAX_HEADER_VALUE_BYTES);
    assert_eq!(notice.check(), Ok(()));
    // Every value at its bound: `events` at 1,024 bytes, both lineage ids at
    // 256, and the rest at their widest encodings.
    let names: usize = notice::headers::NOTICE.iter().map(|n| n.len()).sum();
    let descent = 256 + 1 + 20 + 1 + 64;
    let widest = names + 1024 + 4 * 20 + 3 + 256 + 32 + 64 + 5 * 20 + 256 + descent;
    assert_eq!(widest, 2280);
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
                    crate::view::view_get_codes::UNKNOWN_SESSION,
                    crate::view::view_get_codes::UNKNOWN_VERSION
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

#[test]
fn message_ids_name_their_event_so_equal_numbers_never_collide() {
    let file = file();
    let session_ref = str_of(&file, "session_ref");
    for case in vectors::cases(&file, "notices") {
        let notice = AdvanceNotice::from_headers(&headers(&case["headers"])).unwrap();
        assert_eq!(
            notice.message_id(),
            case["message_id"].as_str().unwrap(),
            "{}",
            name(case)
        );
    }
    for case in vectors::cases(&file, "view_ready") {
        let ready = ViewReady::from_headers(&headers(&case["headers"])).unwrap();
        assert_eq!(ready.message_id(), case["message_id"].as_str().unwrap());
    }
    // The event stream deduplicates ids across all its subjects, so a notice
    // and a view-ready event with the same number must still differ.
    let collision = &file["id_collision"];
    let notice = AdvanceNotice::from_headers(&headers(&collision["notice_headers"])).unwrap();
    let ready = ViewReady::from_headers(&headers(&collision["view_ready_headers"])).unwrap();
    assert_eq!(notice.seq, ready.view_version);
    assert_eq!(
        notice.message_id(),
        collision["notice_id"].as_str().unwrap()
    );
    assert_eq!(
        ready.message_id(),
        collision["view_ready_id"].as_str().unwrap()
    );
    assert_ne!(notice.message_id(), ready.message_id());
    assert!(notice.message_id().starts_with(session_ref));
}

/// The fork-descent cases: each names the lineage the provider holds, the
/// content digests it recorded for that lineage, and the notice that moves
/// the session to a new lineage, with the outcome the provider must reach.
fn check_descent(outcomes: &[&str]) -> usize {
    let file = file();
    let mut checked = 0;
    for case in vectors::cases(&file, "descent") {
        let expected = case["outcome"].as_str().unwrap();
        if !outcomes.iter().any(|o| expected.starts_with(o)) {
            continue;
        }
        let name = name(case);
        let notice = AdvanceNotice::from_headers(&headers(&case["headers"]))
            .unwrap_or_else(|p| panic!("{name}: {p:?}"));
        let recorded: BTreeMap<u64, String> =
            serde_json::from_value::<Vec<(u64, String)>>(case["recorded_digests"].clone())
                .unwrap()
                .into_iter()
                .collect();
        let outcome = descent_outcome(
            case["held_lineage"].as_str().unwrap(),
            |ordinal| recorded.get(&ordinal).cloned(),
            notice.descends_from.as_ref(),
        );
        let spelled = match outcome {
            DescentOutcome::KeepThrough(ordinal) => format!("keep_through:{ordinal}"),
            DescentOutcome::Resync => "resync".to_owned(),
        };
        assert_eq!(spelled, expected, "{name}");
        checked += 1;
    }
    checked
}

#[test]
fn fork_descent_with_matching_digest_keeps_state() {
    assert_eq!(check_descent(&["keep_through"]), 2);
}

#[test]
fn fork_descent_without_a_matching_digest_resyncs() {
    assert_eq!(check_descent(&["resync"]), 4);
}

#[test]
fn descends_from_parses_from_the_right() {
    let digest = "a".repeat(64);
    let descent = NoticeDescent::parse(&format!("run:fork:7:42:{digest}")).unwrap();
    assert_eq!(
        descent,
        NoticeDescent::new("run:fork:7", 42, digest.clone())
    );
    assert_eq!(descent.encode(), format!("run:fork:7:42:{digest}"));
    for bad in [
        format!("42:{digest}"),
        format!(":42:{digest}"),
        format!("lin:042:{digest}"),
        format!("lin:42:{}", "A".repeat(64)),
        format!("lin a:42:{digest}"),
        "lin:42".to_owned(),
    ] {
        assert_eq!(NoticeDescent::parse(&bad), None, "{bad}");
    }
}

#[test]
fn ceiling_overflow_round_trips_with_and_without_a_limit() {
    let file = file();
    for case in vectors::cases(&file["ride"], "overflow_requests") {
        let name = name(case);
        let request = vectors::round_trip::<RideRequest>(name, &case["params"]);
        assert_eq!(request.check(), Ok(()), "{name}");
        // Rides on other causes are covered by
        // `overflow_on_a_non_ceiling_ride_is_ignored`.
        if request.cause == RideCause::Ceiling {
            let expected: Option<Overflow> =
                serde_json::from_value(case["ceiling_overflow"].clone()).unwrap();
            assert_eq!(request.ceiling_overflow(), expected.as_ref(), "{name}");
        }
    }
    let session_ref = str_of(&file, "session_ref");
    let limited = RideRequest::new(session_ref, RideCause::Ceiling, 30000)
        .with_overflow(Overflow::new().with_reported_limit_tokens(200000));
    assert_eq!(
        serde_json::to_value(&limited).unwrap(),
        vectors::cases(&file["ride"], "overflow_requests")[0]["params"]
    );
    let unparsed =
        RideRequest::new(session_ref, RideCause::Ceiling, 30000).with_overflow(Overflow::new());
    assert_eq!(
        serde_json::to_value(&unparsed).unwrap(),
        vectors::cases(&file["ride"], "overflow_requests")[1]["params"]
    );
}

#[test]
fn overflow_is_absent_by_default_and_never_encoded_as_null() {
    let file = file();
    let request = RideRequest::new(str_of(&file, "session_ref"), RideCause::Ceiling, 30000);
    assert_eq!(request.overflow, None);
    let encoded = serde_json::to_value(&request).unwrap();
    assert!(encoded.get("overflow").is_none(), "{encoded}");
    let empty = serde_json::to_value(Overflow::new()).unwrap();
    assert_eq!(empty, serde_json::json!({}));
}

#[test]
fn overflow_on_a_non_ceiling_ride_is_ignored() {
    let file = file();
    for case in vectors::cases(&file["ride"], "overflow_requests") {
        let request: RideRequest = serde_json::from_value(case["params"].clone()).unwrap();
        if request.cause != RideCause::Ceiling {
            assert!(request.overflow.is_some(), "{}", name(case));
            assert_eq!(request.ceiling_overflow(), None, "{}", name(case));
            assert!(case["ceiling_overflow"].is_null(), "{}", name(case));
        }
    }
    let session_ref = str_of(&file, "session_ref");
    for cause in [
        RideCause::TtlExpired,
        RideCause::ModelChange,
        RideCause::SurfaceChange,
        RideCause::Unknown("provider_switch".into()),
    ] {
        let request = RideRequest::new(session_ref, cause.clone(), 1500)
            .with_overflow(Overflow::new().with_reported_limit_tokens(128000));
        assert_eq!(request.check(), Ok(()), "{cause:?}");
        assert_eq!(request.ceiling_overflow(), None, "{cause:?}");
    }
}

#[test]
fn lost_runner_state_continues_under_a_new_lineage_and_resyncs() {
    let file = file();
    let case = &file["state_loss"];
    let notices: Vec<AdvanceNotice> = case["notices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|headers_json| AdvanceNotice::from_headers(&headers(headers_json)).unwrap())
        .collect();
    let (before, after) = (&notices[0], &notices[1]);
    assert_eq!(before.session_ref, after.session_ref);
    assert_eq!(before.lineage_id, case["held_lineage"].as_str().unwrap());
    assert_ne!(after.lineage_id, before.lineage_id);
    assert_eq!(after.descends_from, None);
    assert!(after.applied_view_version.is_none());
    assert!(!after
        .events
        .iter()
        .any(|event| matches!(event, Event::RevertBelow(_))));
    let recorded: BTreeMap<u64, String> =
        serde_json::from_value::<Vec<(u64, String)>>(case["recorded_digests"].clone())
            .unwrap()
            .into_iter()
            .collect();
    let outcome = descent_outcome(
        &before.lineage_id,
        |ordinal| recorded.get(&ordinal).cloned(),
        after.descends_from.as_ref(),
    );
    assert_eq!(outcome, DescentOutcome::Resync);
    assert_eq!(case["outcome"], "resync");
}

fn apply_view(key: &str) -> CompactionView {
    let file = file();
    let case = &file["view_apply"][key];
    vectors::round_trip::<CompactionView>(name(case), &case["view"])
}

/// The marked view's digest, computed by the generator, frames `apply` and
/// the mark: this crate computes the same value, it differs from the same
/// view unmarked, and the builder reproduces the vector.
#[test]
fn a_marked_view_digest_covers_its_apply_mark() {
    let marked = apply_view("marked");
    let unmarked = apply_view("unmarked");
    assert_eq!(marked.apply.as_deref(), Some(view::apply_marks::NEXT_STEP));
    assert_eq!(marked.compute_digest(), marked.digest);
    assert_eq!(marked.check(), Ok(()));
    assert_ne!(marked.digest, unmarked.digest);
    let built = unmarked.clone().with_apply(view::apply_marks::NEXT_STEP);
    assert_eq!(built, marked);
}

/// A view without a mark is digested exactly as before the mark existed: the
/// generator frames nothing for it, and removing the mark from the marked
/// view gives back the unmarked view's digest. Every older view vector keeps
/// its digest too (`views_check_and_assemble_as_recorded`).
#[test]
fn an_unmarked_view_keeps_the_digest_it_had_before_the_mark() {
    let unmarked = apply_view("unmarked");
    assert_eq!(unmarked.apply, None);
    assert_eq!(unmarked.compute_digest(), unmarked.digest);
    assert_eq!(unmarked.check(), Ok(()));
    let encoded = serde_json::to_value(&unmarked).unwrap();
    assert!(encoded.get("apply").is_none());
    let mut stripped = apply_view("marked");
    stripped.apply = None;
    assert_eq!(stripped.compute_digest(), unmarked.digest);
}

/// A mark this crate does not define still decodes, round-trips and passes
/// the view's own checks: refusing it with `unsupported_apply` is the
/// runner's decision, made against the marks it listed in Setup.
#[test]
fn a_view_with_an_unknown_apply_mark_still_decodes() {
    let unknown = apply_view("unknown_mark");
    assert_eq!(unknown.apply.as_deref(), Some("when_idle"));
    assert_eq!(unknown.check(), Ok(()));
    assert_eq!(
        notice::refusal_reasons::UNSUPPORTED_APPLY,
        "unsupported_apply"
    );
    let refused = Event::ViewRefused {
        view_version: unknown.view_version,
        reason: notice::refusal_reasons::UNSUPPORTED_APPLY.into(),
    };
    assert_eq!(Event::parse(&refused.token()), Ok(refused));
}

/// Setup's `view_apply` round-trips and matches the builder; it is omitted
/// when empty, and a name the provider does not know still decodes.
#[test]
fn setup_view_apply_round_trips() {
    let file = file();
    let case = &vectors::cases(&file["view_apply"], "setup_requests")[0];
    let request = vectors::round_trip::<SetupRequest>(name(case), &case["request"]);
    assert_eq!(request.view_apply, vec![view::apply_marks::NEXT_STEP]);
    let composition = case["request"]["composition"].as_object().unwrap().clone();
    let built = SetupRequest::new(
        "ses_1",
        "demo-harness",
        "req-setup-2",
        composition,
        "acme/model-1",
        1790000000000,
    )
    .with_view_apply(vec![view::apply_marks::NEXT_STEP.into()]);
    assert_eq!(built, request);
    let mut without = case["request"].clone();
    without.as_object_mut().unwrap().remove("view_apply");
    let plain = vectors::round_trip::<SetupRequest>("no view_apply", &without);
    assert!(plain.view_apply.is_empty());
    let mut unknown = case["request"].clone();
    unknown["view_apply"] = serde_json::json!(["next_step", "when_idle"]);
    let decoded: SetupRequest = serde_json::from_value(unknown).unwrap();
    assert_eq!(decoded.view_apply, vec!["next_step", "when_idle"]);
}

#[test]
fn provider_route_identity_vectors_keep_conversation_and_child_scopes_distinct() {
    let file = vectors::load("route-identity.json");
    for case in vectors::cases(&file, "identity_cases") {
        let route = &case["incoming_route"]["bind"];
        let setup = &case["setup"];
        let expected = &case["expected"];
        let setup_session = setup["session"].as_str().unwrap();
        let route_session = route["session"].as_str().unwrap();

        if expected["setup"] == "refuse" {
            assert!(!route_session.is_empty(), "{}", name(case));
            assert_ne!(route_session, setup_session, "{}", name(case));
            assert_eq!(
                expected["code"],
                "setup_identity_mismatch",
                "{}",
                name(case)
            );
        }

        if let Some(reverse_bind) = expected.get("reverse_bind") {
            assert_eq!(reverse_bind["harness"], setup["harness"], "{}", name(case));
            assert_eq!(reverse_bind["session"], setup["session"], "{}", name(case));
            assert_eq!(
                reverse_bind["project_root"],
                route["project_root"],
                "{}",
                name(case)
            );
            assert!(
                route_session.is_empty() || route_session == setup_session,
                "{}",
                name(case)
            );
        }

        if let Some(child_bind) = expected.get("child_bind") {
            assert_eq!(child_bind["harness"], setup["harness"], "{}", name(case));
            assert_eq!(
                child_bind["session"],
                case["child_session"],
                "{}",
                name(case)
            );
            assert_eq!(
                child_bind["project_root"],
                route["project_root"],
                "{}",
                name(case)
            );
            assert_ne!(child_bind["session"], setup["session"], "{}", name(case));
            assert_eq!(expected["forbidden_child_bind_session"], setup["session"]);
        }

        if expected.get("runner_authenticated") == Some(&Value::Bool(false)) {
            assert_eq!(
                case["incoming_route"]["principal"]["daemon_attested"],
                false
            );
            assert_ne!(
                case["incoming_route"]["principal"]["value"],
                route["harness"]
            );
        }
    }
    for fault in vectors::cases(&file, "provider_faults") {
        let route = &fault["incoming_route"];
        let setup = &fault["setup"];
        let expected = &fault["expected_reverse_bind"];
        let faulty = &fault["faulty_reverse_bind"];
        assert_eq!(expected["harness"], setup["harness"], "{}", name(fault));
        assert_eq!(expected["session"], setup["session"], "{}", name(fault));
        assert_eq!(
            expected["project_root"],
            route["project_root"],
            "{}",
            name(fault)
        );
        assert_ne!(faulty, expected, "{}", name(fault));
    }
}

/// A reply body is the bare answer object, never `{"result": ...}`, and a
/// refusal's fields other than `code` and `message` live inside `detail`.
#[test]
fn replies_are_bare_answers_and_refusals_keep_detail_inside() {
    let file = vectors::load("reply-shape.json");
    for case in vectors::cases(&file, "replies") {
        assert!(case["body"].get("result").is_none(), "{}", name(case));
        let answer = vectors::round_trip::<RideAnswer>(name(case), &case["body"]);
        assert_eq!(answer, RideAnswer::nothing(), "{}", name(case));
    }
    for case in vectors::cases(&file, "refusals") {
        let body = case["body"].as_object().unwrap();
        assert!(
            body.keys()
                .all(|key| ["code", "message", "detail"].contains(&key.as_str())),
            "{}",
            name(case)
        );
        vectors::round_trip::<crate::errors::ErrorBody>(name(case), &case["body"]);
    }
}
