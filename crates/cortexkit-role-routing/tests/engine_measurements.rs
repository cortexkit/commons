use cortexkit_role_routing::{
    capabilities,
    describe::{check_describe, DescribeProblem, RoleDescribe},
    engine_measurements::*,
    ops,
    select_engine::*,
};
use serde_json::{json, Value};

fn eval(model: &str, metric: &str, score: f64, set: &str, time: u64) -> EngineEvalRequest {
    EngineEvalRequest::new(
        Kind::Transcribe,
        EngineIdentity::new("example", model),
        metric,
        score,
        EvaluationSet::new(set, 10),
        time,
    )
}

fn candidates() -> Vec<EngineCandidate<EvalScore, u64>> {
    ["a", "b", "c"].into_iter().map(|model| {
        let engine: Engine = serde_json::from_value(json!({"kind":"transcribe", "provider":"example", "model":model, "local":false,"formats":["audio/wav"],"latency_class":"interactive"})).unwrap();
        EngineCandidate::new("example-media", engine)
    }).collect()
}

fn request() -> SelectEngineRequest {
    SelectEngineRequest::new(
        "example.transcribe",
        Kind::Transcribe,
        EngineRequirements::new(),
        Privacy::RemoteAllowed,
    )
}

#[test]
fn engine_measurements_metric_set_recency_and_identity_are_comparable() {
    let mut candidates = candidates();
    let mut request = request();
    let evaluations = vec![
        eval("a", "wer", 0.8, "canonical", 200),
        eval("b", "wer", 0.9, "canonical", 200),
        eval("b", "wer", 0.1, "canonical", 100),
        eval("c", "wer", 1.0, "unrelated", 100),
        eval("a", "named_term_recall", 1.0, "canonical", 200),
    ];
    apply_evaluations(&request, &mut candidates, &evaluations);
    assert_eq!(
        select_engine(&request, &candidates).unwrap().engine.model,
        "b"
    );
    assert_eq!(candidates[2].accuracy, None);
    request.requirements.metric = Some("named_term_recall".into());
    apply_evaluations(&request, &mut candidates, &evaluations);
    assert_eq!(
        select_engine(&request, &candidates).unwrap().engine.model,
        "a"
    );
    request.requirements.metric = None;
    candidates[1].engine.model = "b-new".into();
    apply_evaluations(&request, &mut candidates, &evaluations);
    assert_eq!(candidates[1].accuracy, None);
    let mut foreign = eval("b-new", "wer", 1.0, "canonical", 200);
    foreign.engine.provider = "other-provider".into();
    apply_evaluations(&request, &mut candidates, &[foreign]);
    assert!(candidates
        .iter()
        .all(|candidate| candidate.accuracy.is_none()));
}

#[test]
fn engine_measurements_newest_set_bytewise_ties_and_filtered_candidates() {
    let mut candidates = candidates();
    let request = request();
    let mut evaluations = vec![
        eval("a", "wer", 1.0, "old", 10),
        eval("b", "wer", 0.1, "new", 20),
    ];
    apply_evaluations(&request, &mut candidates, &evaluations);
    assert_eq!(
        select_engine(&request, &candidates).unwrap().engine.model,
        "b"
    );
    evaluations.push(eval("a", "wer", 0.2, "a-set", 20));
    apply_evaluations(&request, &mut candidates, &evaluations);
    assert_eq!(
        select_engine(&request, &candidates).unwrap().engine.model,
        "a"
    );
    candidates[0].fenced = true;
    apply_evaluations(&request, &mut candidates, &evaluations);
    assert_eq!(
        select_engine(&request, &candidates).unwrap().engine.model,
        "b"
    );
    let local = EngineCandidate::caller_local(0, candidates[1].engine.clone());
    candidates.push(local);
    apply_evaluations(&request, &mut candidates, &evaluations);
    assert_eq!(candidates.last().unwrap().accuracy, None);
}

#[test]
fn engine_measurements_kind_without_default_stays_unmeasured() {
    let mut candidates = candidates();
    for candidate in &mut candidates {
        candidate.engine.kind = Kind::Image;
    }
    let mut request = request();
    request.kind = Kind::Image;
    let mut evaluation = eval("b", "wer", 1.0, "canonical", 10);
    evaluation.kind = Kind::Image;
    apply_evaluations(&request, &mut candidates, &[evaluation.clone()]);
    assert!(candidates
        .iter()
        .all(|candidate| candidate.accuracy.is_none()));
    request.requirements.metric = Some("wer".into());
    apply_evaluations(&request, &mut candidates, &[evaluation]);
    assert_eq!(
        select_engine(&request, &candidates).unwrap().engine.model,
        "b"
    );
}

#[test]
fn engine_measurements_parsers_scores_paths_and_open_strings() {
    for score in [-0.01, 1.01, f64::NAN, f64::INFINITY] {
        assert_eq!(EvalScore::new(score).unwrap_err().detail["field"], "score");
    }
    for score in [0.0, 1.0] {
        assert_eq!(EvalScore::new(score).unwrap().get(), score);
    }
    assert_eq!(EvalScore::new(-0.0).unwrap(), EvalScore::new(0.0).unwrap());
    let body = serde_json::to_value(eval("a", "future_metric", 0.5, "canonical", 10)).unwrap();
    assert_eq!(
        parse_engine_eval_request(&body).unwrap().metric,
        "future_metric"
    );
    for (path, value) in [
        ("/engine/model", json!(1)),
        ("/set/items", json!(-1)),
        ("/score", json!(2)),
        ("/measured_at_ms", json!(-1)),
    ] {
        let mut bad = body.clone();
        *bad.pointer_mut(path).unwrap() = value;
        assert_eq!(
            parse_engine_eval_request(&bad).unwrap_err().detail["field"],
            path.trim_start_matches('/').replace('/', ".")
        );
    }
    let mut bad = body;
    bad["engine"]["unexpected"] = json!(true);
    assert_eq!(
        parse_engine_eval_request(&bad).unwrap_err().detail["field"],
        "engine.unexpected"
    );
    let report = json!({"decision_id":"d","kind":"transcribe","engine":{"provider":"example","model":"a"},"outcome":"future_outcome","input":{"duration_ms":1000,"bytes":100,"chars":10},"wall_ms":250,"first_output_ms":20});
    let mut parsed = parse_engine_report_request(&report).unwrap();
    assert_eq!(parsed.realtime_factor(), Some(0.25));
    parsed.input.duration_ms = Some(0);
    assert_eq!(parsed.realtime_factor(), None);
    parsed.input.duration_ms = None;
    assert_eq!(parsed.realtime_factor(), None);
    let mut bad = report;
    bad["input"]["duration_ms"] = Value::String("1000".into());
    assert_eq!(
        parse_engine_report_request(&bad).unwrap_err().detail["field"],
        "input.duration_ms"
    );
}

#[test]
fn engine_measurements_authorization_uses_attested_pair_and_kind() {
    let candidates = candidates();
    let engine = &candidates[0].engine;
    let identity = EngineIdentity::new("example", "a");
    assert!(
        check_engine_authorization("owner", &Kind::Transcribe, &identity, [("owner", engine)])
            .is_ok()
    );
    for (caller, kind, identity) in [
        ("foreign", Kind::Transcribe, identity.clone()),
        ("owner", Kind::Image, identity.clone()),
        (
            "owner",
            Kind::Transcribe,
            EngineIdentity::new("example", "a-new"),
        ),
    ] {
        assert_eq!(
            check_engine_authorization(caller, &kind, &identity, [("owner", engine)])
                .unwrap_err()
                .code
                .as_str(),
            "engine_not_declared_by_caller"
        );
    }
    assert!(check_engine_authorization("owner", &Kind::Transcribe, &identity, []).is_err());
}

#[test]
fn engine_measurements_groups_require_select_engine() {
    for (group, op) in [
        (capabilities::ENGINE_REPORTS, ops::ROUTE_ENGINE_REPORT),
        (capabilities::ENGINE_EVALS, ops::ROUTE_ENGINE_EVAL),
    ] {
        let mut describe = RoleDescribe::new(vec![], false).with_capability(group);
        describe.ops.push(op.into());
        assert_eq!(
            check_describe(&describe),
            Err(DescribeProblem::MissingCapabilityDependency {
                group,
                requires: capabilities::SELECT_ENGINE
            })
        );
        describe
            .capabilities
            .push(capabilities::SELECT_ENGINE.into());
        describe.ops.push(ops::ROUTE_SELECT_ENGINE.into());
        assert_eq!(check_describe(&describe), Ok(()));
    }
}
