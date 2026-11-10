use cortexkit_role_media::bias::{BiasLimit, BiasUnit};
use cortexkit_role_routing::{
    describe::{check_describe, DescribeProblem, RoleDescribe as Describe},
    errors::{NoEngineDetail, RefusalCode},
    parse_engine_declarations,
    select_engine::{
        check_answer, meets_requirements, parse_select_engine_request, select_engine, Engine,
        EngineCandidate, EngineFilter, EngineRequirements, Kind, LatencyClass, Privacy,
        SelectEngineAnswer, SelectEngineRequest, SelectedEngine,
    },
};
use serde_json::{json, Value};

fn engine() -> Engine {
    Engine::new(
        Kind::Transcribe,
        "vendor".into(),
        "model".into(),
        false,
        vec!["audio/L16;rate=16000;channels=1".into()],
        LatencyClass::Interactive,
    )
}

fn request() -> Value {
    json!({"consumer":"app.speech", "kind":"transcribe", "requirements":{}, "privacy":"remote_allowed"})
}

fn parsed() -> SelectEngineRequest {
    parse_select_engine_request(&request()).unwrap()
}

fn candidate(module: &str) -> EngineCandidate<i64, u64> {
    EngineCandidate::new(module, engine())
}

#[test]
fn select_engine_declarations_are_atomic_and_use_every_per_engine_check() {
    let good = serde_json::to_value(engine()).unwrap();
    assert_eq!(
        parse_engine_declarations(&json!({"engines":[good.clone()]}))
            .unwrap()
            .engines,
        vec![engine()]
    );
    assert!(parse_engine_declarations(&json!({"engines":[]}))
        .unwrap()
        .engines
        .is_empty());
    // The slot may carry members besides engines. Those are ignored; a missing
    // engines member is still invalid_params at that path.
    assert_eq!(
        parse_engine_declarations(&json!({"engines":[good.clone()], "runner":{}}))
            .unwrap()
            .engines,
        vec![engine()]
    );
    // Engine identity includes the provider: one module may declare the same
    // kind and model served by a vendor and by a gateway.
    let mut gateway = good.clone();
    gateway["provider"] = json!("gateway");
    assert_eq!(
        parse_engine_declarations(&json!({"engines":[good.clone(), gateway]}))
            .unwrap()
            .engines
            .len(),
        2
    );
    let missing = parse_engine_declarations(&json!({
        "groups": ["jobs"],
        "jobs_survive_restart": false,
    }))
    .unwrap_err();
    assert_eq!(
        (missing.code.as_str(), missing.field.as_str()),
        ("invalid_params", "engines")
    );
    let mut cases = vec![(
        json!({"engines":[good.clone(), good.clone()]}),
        "duplicate_engine",
        "engines[1].model".to_owned(),
    )];
    for (field, value, code, path) in [
        ("formats", json!([]), "formats_empty", "formats"),
        (
            "provider",
            json!("é".repeat(257)),
            "fact_too_long",
            "provider",
        ),
        ("model", json!("é".repeat(257)), "fact_too_long", "model"),
        (
            "languages",
            json!(["é".repeat(257)]),
            "fact_too_long",
            "languages[0]",
        ),
        ("formats", json!([false]), "invalid_params", "formats[0]"),
        (
            "max_input_bytes",
            json!(-1),
            "invalid_params",
            "max_input_bytes",
        ),
        ("extra", json!(1), "unknown_member", "extra"),
        (
            "bias_limit",
            json!({"unit":"phrase", "max":0}),
            "bias_limit_invalid",
            "bias_limit.max",
        ),
        (
            "bias_limit",
            json!({"unit":"phrase", "max":-1}),
            "bias_limit_invalid",
            "bias_limit.max",
        ),
        (
            "bias_limit",
            Value::Null,
            "bias_limit_invalid",
            "bias_limit",
        ),
        (
            "bias_limit",
            json!({"unit":"phrase", "max":1,"other":1}),
            "unknown_member",
            "bias_limit.other",
        ),
    ] {
        let mut bad = good.clone();
        bad[field] = value;
        cases.push((
            json!({"engines":[good.clone(),bad]}),
            code,
            format!("engines[1].{path}"),
        ));
    }
    let mut local_realtime = good.clone();
    local_realtime["kind"] = json!("realtime");
    local_realtime["local"] = json!(true);
    cases.push((
        json!({"engines":[good.clone(),local_realtime]}),
        "local_realtime_engine",
        "engines[1].local".to_owned(),
    ));
    for (slot, code, field) in cases {
        let error = parse_engine_declarations(&slot).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.field.as_str()),
            (code, field.as_str())
        );
    }
    let mut boundary = good;
    boundary["model"] = json!("é".repeat(256));
    boundary["kind"] = json!("future_kind");
    boundary["latency_class"] = json!("future_latency");
    assert!(parse_engine_declarations(&json!({"engines":[boundary]})).is_ok());
}

#[test]
fn describe_shaped_media_slot_decodes_every_engine() {
    let speech = engine();
    let picture = Engine::new(
        Kind::Image,
        "studio".into(),
        "canvas".into(),
        false,
        vec!["image/png".into()],
        LatencyClass::Batch,
    );
    let slot = json!({
        "groups": ["jobs", "usage_feed"],
        "jobs_survive_restart": false,
        "engines": [
            serde_json::to_value(&speech).unwrap(),
            serde_json::to_value(&picture).unwrap(),
        ],
    });
    assert_eq!(
        parse_engine_declarations(&slot).unwrap().engines,
        vec![speech, picture]
    );
}

#[test]
fn select_engine_rejects_every_malformed_requirement_and_unknown_member() {
    for (field, value) in [
        ("language", json!(1)),
        ("voice", json!(false)),
        ("format", json!([])),
        ("duration_ms", json!(-1)),
        ("input_bytes", json!(1.5)),
        ("reference_formats", json!([1])),
        ("bias", json!("true")),
        ("size", json!(0)),
        ("resolution", json!({})),
        ("quality", json!(0)),
        ("latency_class", json!("unknown")),
        ("other", json!(true)),
    ] {
        let mut value_request = request();
        value_request["requirements"][field] = value;
        let refusal = parse_select_engine_request(&value_request).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequirements, "{field}");
        assert_eq!(refusal.detail["field"], field);
    }
    for value in [Value::Null, json!([]), json!(false)] {
        let mut body = request();
        body["requirements"] = value;
        let refusal = parse_select_engine_request(&body).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequirements);
        assert_eq!(refusal.detail["field"], "requirements");
    }
    let mut realtime = request();
    realtime["kind"] = json!("realtime");
    realtime["requirements"] = json!({"format":"audio/wav"});
    let refusal = parse_select_engine_request(&realtime).unwrap_err();
    assert_eq!(refusal.code, RefusalCode::InvalidRequirements);
    assert_eq!(refusal.detail["field"], "format");
    assert!(serde_json::from_value::<SelectEngineRequest>(json!({
        "consumer":"a", "kind":"image", "privacy":"local_only", "requirements":{}, "new":true
    }))
    .is_err());
    assert!(serde_json::from_value::<EngineRequirements>(json!({"new":true})).is_err());
}

#[test]
fn select_engine_request_and_caller_local_errors_name_the_path() {
    for (field, value) in [
        ("consumer", json!("")),
        ("privacy", json!("maybe")),
        ("extra", json!(1)),
    ] {
        let mut body = request();
        body[field] = value;
        let refusal = parse_select_engine_request(&body).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequest);
        assert_eq!(refusal.detail["field"], field);
    }
    let mut body = request();
    body["caller_local"] = json!([engine()]);
    let refusal = parse_select_engine_request(&body).unwrap_err();
    assert_eq!(refusal.code, RefusalCode::InvalidRequest);
    assert_eq!(refusal.detail["field"], "caller_local[0].local");
    let mut local = engine();
    local.local = true;
    body["caller_local"] = json!([local.clone()]);
    assert_eq!(
        parse_select_engine_request(&body).unwrap().caller_local,
        Some(vec![local.clone()])
    );
    body["caller_local"] = json!([local.clone(), local]);
    let refusal = parse_select_engine_request(&body).unwrap_err();
    assert_eq!(refusal.detail["field"], "caller_local[1].model");
    body["caller_local"][0]["formats"] = json!([]);
    assert_eq!(
        parse_select_engine_request(&body).unwrap_err().detail["field"],
        "caller_local[0].formats"
    );
    body = request();
    body["kind"] = json!("future_kind");
    let parsed = parse_select_engine_request(&body).unwrap();
    assert_eq!(serde_json::to_value(&parsed).unwrap(), body);
}

#[test]
fn select_engine_every_requirement_is_matched_and_missing_facts_fail() {
    let mut full = engine()
        .with_languages(vec!["en".into()])
        .with_voices(vec!["a".into()])
        .with_max_duration_ms(10)
        .with_max_input_bytes(20)
        .with_reference_formats(vec!["image/png".into()])
        .with_bias_limit(BiasLimit::new(BiasUnit::Phrase, 3))
        .with_sizes(vec!["small".into()])
        .with_resolutions(vec!["high".into()])
        .with_qualities(vec!["best".into()]);
    let requirements = json!({"language":"EN-gb", "voice":"a", "format":"AUDIO/l16;rate=16000;channels=1", "duration_ms":10, "input_bytes":20, "reference_formats":["IMAGE/PNG"], "bias":true, "size":"small", "resolution":"high", "quality":"best", "latency_class":"batch"});
    for (field, value) in requirements.as_object().unwrap() {
        let req: EngineRequirements = serde_json::from_value(json!({(field):value})).unwrap();
        assert!(meets_requirements(&full, &req), "{field}");
        let mut without = serde_json::to_value(&full).unwrap();
        let fact = match field.as_str() {
            "language" => "languages",
            "voice" => "voices",
            "format" => "formats",
            "duration_ms" => "max_duration_ms",
            "input_bytes" => "max_input_bytes",
            "bias" => "bias_limit",
            "size" => "sizes",
            "resolution" => "resolutions",
            "quality" => "qualities",
            other => other,
        };
        if fact == "formats" {
            without[fact] = json!(["audio/wav"]);
        } else if fact == "latency_class" {
            without[fact] = json!("unknown");
        } else {
            without.as_object_mut().unwrap().remove(fact);
        }
        let without: Engine = serde_json::from_value(without).unwrap();
        assert!(!meets_requirements(&without, &req), "{field}");
    }
    for (field, value) in [
        ("language", json!("eng")),
        ("voice", json!("b")),
        ("format", json!("audio/L16;rate=8000;channels=1")),
        ("duration_ms", json!(11)),
        ("input_bytes", json!(21)),
        ("reference_formats", json!(["image/png", "image/jpeg"])),
        ("size", json!("large")),
        ("resolution", json!("low")),
        ("quality", json!("worst")),
        ("latency_class", json!("realtime")),
    ] {
        let req: EngineRequirements = serde_json::from_value(json!({(field):value})).unwrap();
        assert!(!meets_requirements(&full, &req), "{field}");
    }
    full.languages = Some(vec!["en-GB".into()]);
    assert!(!meets_requirements(
        &full,
        &EngineRequirements::new().with_language("en".into())
    ));
    full.latency_class = LatencyClass::Unknown("new".into());
    full.bias_limit = None;
    assert!(meets_requirements(
        &full,
        &EngineRequirements::new().with_bias(false)
    ));
}

#[test]
fn select_engine_filters_whole_sets_in_order_and_local_engines_ignore_fences() {
    let mut request = parsed();
    let check_filter =
        |candidates: &[EngineCandidate<i64, u64>], req: &SelectEngineRequest, expected| {
            let refusal = select_engine(req, candidates).unwrap_err();
            assert_eq!(refusal.code, RefusalCode::NoEngine);
            assert_eq!(
                refusal.detail_as::<NoEngineDetail>().unwrap().filter,
                expected
            );
            assert_eq!(refusal.wait_until_ms(), None);
        };
    check_filter(&[], &request, EngineFilter::Kind);
    let mut other = candidate("remote");
    other.engine.kind = Kind::Image;
    check_filter(&[other], &request, EngineFilter::Kind);
    request.requirements = EngineRequirements::new().with_input_bytes(1);
    check_filter(&[candidate("remote")], &request, EngineFilter::Requirements);
    request.requirements = EngineRequirements::new();
    request.privacy = Privacy::LocalOnly;
    check_filter(&[candidate("remote")], &request, EngineFilter::Privacy);
    request.privacy = Privacy::RemoteAllowed;
    check_filter(
        &[candidate("remote").with_fenced(true)],
        &request,
        EngineFilter::Reachability,
    );
    let mut local = engine();
    local.local = true;
    for privacy in [Privacy::RemoteAllowed, Privacy::LocalOnly] {
        request.privacy = privacy;
        for local_candidate in [
            EngineCandidate::<i64, u64>::new("local", local.clone()),
            EngineCandidate::caller_local(0, local.clone()),
        ] {
            let candidates = [
                candidate("remote").with_fenced(true),
                local_candidate.with_fenced(true),
            ];
            assert!(select_engine(&request, &candidates).unwrap().engine.local);
        }
    }
    request.privacy = Privacy::RemoteAllowed;
    let candidates = [
        candidate("remote").with_accuracy(100),
        EngineCandidate::new("local", local).with_accuracy(90),
    ];
    assert!(!select_engine(&request, &candidates).unwrap().engine.local);
    let mut insufficient = candidate("a");
    insufficient.engine.kind = Kind::Image;
    request.requirements = EngineRequirements::new().with_input_bytes(1);
    check_filter(
        &[insufficient, candidate("b")],
        &request,
        EngineFilter::Requirements,
    );
}

#[test]
fn select_engine_ranking_prefers_measured_accuracy_then_known_cost_and_exact_ties() {
    let request = parsed();
    let pick = |candidates: &[EngineCandidate<i64, u64>]| {
        select_engine(&request, candidates)
            .unwrap()
            .selected
            .clone()
    };
    let measured = candidate("measured").with_accuracy(0).with_cost(100);
    let unmeasured = candidate("unmeasured").with_cost(0);
    assert_eq!(pick(&[unmeasured, measured.clone()]), measured.selected);
    let accurate = candidate("accurate").with_accuracy(2).with_cost(500);
    assert_eq!(
        pick(&[measured.clone(), accurate.clone()]),
        accurate.selected
    );
    let costly = candidate("costly").with_accuracy(2).with_cost(100);
    let cheap = candidate("cheap").with_accuracy(2).with_cost(1);
    let unknown_cost = candidate("a").with_accuracy(2);
    assert_eq!(pick(&[unknown_cost, costly, cheap.clone()]), cheap.selected);
    let a = candidate("a");
    let z = candidate("z");
    assert_eq!(pick(&[z, a.clone()]), a.selected);
    let mut provider = candidate("a");
    provider.engine.provider = "a".into();
    provider.selected.provider = "a".into();
    assert_eq!(pick(&[a.clone(), provider.clone()]), provider.selected);
    let mut model = provider.clone();
    model.engine.model = "a".into();
    model.selected.model = "a".into();
    assert_eq!(pick(&[provider, model.clone()]), model.selected);
    let mut local = engine();
    local.local = true;
    let caller = EngineCandidate::caller_local(0, local.clone());
    assert_eq!(pick(&[a, caller.clone()]), caller.selected);
    let later = EngineCandidate::caller_local(1, local);
    assert_eq!(pick(&[later, caller.clone()]), caller.selected);
}

#[test]
fn select_engine_answers_enforce_module_and_caller_local_member_matrix() {
    let mut req = parsed();
    let remote =
        SelectEngineAnswer::new("decision", 600_000, SelectedEngine::new("media", &engine()));
    assert!(check_answer(&req, &remote).is_ok());
    let mut invalid = remote.clone();
    invalid.engine.index = Some(0);
    assert!(check_answer(&req, &invalid).is_err());
    invalid = remote.clone();
    invalid.engine.module = None;
    assert!(check_answer(&req, &invalid).is_err());
    req.privacy = Privacy::LocalOnly;
    assert!(check_answer(&req, &remote).is_err());
    let mut local = engine();
    local.local = true;
    req.caller_local = Some(vec![local.clone()]);
    let valid = SelectEngineAnswer::new(
        "local-decision",
        600_000,
        SelectedEngine::caller_local(0, &local),
    );
    assert!(check_answer(&req, &valid).is_ok());
    let value = serde_json::to_value(&valid).unwrap();
    assert!(value["engine"].get("module").is_none());
    for (field, value) in [
        ("module", json!("media")),
        ("index", json!(1)),
        ("local", json!(false)),
        ("model", json!("wrong")),
        ("provider", json!("wrong")),
    ] {
        let mut bad = serde_json::to_value(&valid).unwrap();
        bad["engine"][field] = value;
        assert!(
            check_answer(&req, &serde_json::from_value(bad).unwrap()).is_err(),
            "{field}"
        );
    }
    req.requirements = EngineRequirements::new().with_bias(true);
    assert!(check_answer(&req, &valid).is_err());
}

#[test]
fn select_engine_group_and_operation_must_be_declared_together() {
    use cortexkit_role_routing::{capabilities::SELECT_ENGINE, ops::ROUTE_SELECT_ENGINE};
    let mut describe = Describe::new(vec![], false);
    assert!(check_describe(&describe).is_ok());
    describe.capabilities.push(SELECT_ENGINE.into());
    assert_eq!(
        check_describe(&describe),
        Err(DescribeProblem::OpGroupMismatch {
            group: SELECT_ENGINE
        })
    );
    describe.ops.push(ROUTE_SELECT_ENGINE.into());
    assert!(check_describe(&describe).is_ok());
    describe.capabilities.clear();
    assert_eq!(
        check_describe(&describe),
        Err(DescribeProblem::OpGroupMismatch {
            group: SELECT_ENGINE
        })
    );
}
