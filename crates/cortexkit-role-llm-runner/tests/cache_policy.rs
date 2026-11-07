use cortexkit_role_llm_runner::cache::{self, Cache, Hours, KeepWarm, Ttl};
use serde_json::json;

fn request(value: serde_json::Value) -> Cache {
    serde_json::from_value(value).unwrap()
}

#[test]
fn absent_cache_and_absent_mode_mean_warming_off() {
    let absent: Option<Cache> = serde_json::from_value(serde_json::Value::Null).unwrap();
    assert_eq!(absent.unwrap_or_default().keep_warm(), KeepWarm::Off);
    let cache = request(json!({}));
    assert_eq!(cache.ttl, None);
    assert_eq!(cache.key, None);
    assert_eq!(cache.keep_warm(), KeepWarm::Off);
    assert_eq!(serde_json::to_value(&cache).unwrap(), json!({}));
    assert!(cache::check(&cache).is_ok());
    assert!(cache::check(&request(json!({"ttl":"none"}))).is_ok());
}

#[test]
fn cache_shape_refusals_name_exact_plan_and_send_fields() {
    for mode in ["hours", "always", "auto"] {
        let cache = request(json!({"ttl":"none","keep_warm":mode}));
        assert_refusal(&cache, "keep_warm");
    }
    assert_refusal(&request(json!({"keep_warm":"hours"})), "hours");
    for mode in [None, Some("off"), Some("always"), Some("auto")] {
        let mut value = json!({"hours":{"from":"09:00","to":"17:00","tz":"Etc/UTC"}});
        if let Some(mode) = mode {
            value["keep_warm"] = json!(mode);
        }
        assert_refusal(&request(value), "hours");
    }
    for invalid in [
        "9:00",
        "09:0",
        "24:00",
        "00:60",
        "-1:00",
        "00:00:00",
        "aa:bb",
        "０９:00",
        "",
    ] {
        for field in ["from", "to"] {
            let mut value =
                json!({"keep_warm":"hours","hours":{"from":"09:00","to":"17:00","tz":"Etc/UTC"}});
            value["hours"][field] = json!(invalid);
            assert_refusal(&request(value), &format!("hours.{field}"));
        }
    }
    assert_refusal(
        &request(json!({"keep_warm":"hours","hours":{"from":"09:00","to":"09:00","tz":"Etc/UTC"}})),
        "hours.to",
    );
}

fn assert_refusal(cache: &Cache, field: &str) {
    for (result, prefix) in [
        (cache::check(cache), "plan.cache"),
        (cache::check_send(cache), "cache"),
    ] {
        let error = result.expect_err(field);
        assert_eq!(error.code, "invalid_params");
        assert_eq!(error.detail, json!({"field":format!("{prefix}.{field}")}));
    }
}

#[test]
fn cache_key_boundaries_and_wire_round_trips() {
    use cortexkit_role_llm_runner::send::SendRequest;
    let printable: String = (0x21..=0x7e).map(char::from).collect();
    for key in ["!".to_owned(), "~".repeat(128), printable] {
        let value = json!({"key":key});
        let cache = cache::check_send_value(&value).unwrap();
        assert_eq!(cache.key.as_deref(), Some(key.as_str()));
        assert_eq!(serde_json::to_value(&cache).unwrap(), value);
        assert_eq!(cache.ttl, None);
        assert_eq!(cache.keep_warm(), KeepWarm::Off);
        for plan in [false, true] {
            let mut wire = json!({"prompt":"hello","send_id":"cache-key"});
            if plan {
                wire["plan"] = json!({"cache":value});
            } else {
                wire["cache"] = value.clone();
            }
            let send: SendRequest = serde_json::from_value(wire.clone()).unwrap();
            assert!(send.check_cache(true).is_ok());
            assert_eq!(serde_json::to_value(send).unwrap(), wire);
        }
    }
}

#[test]
fn cache_key_refusals_name_exact_fields_for_raw_and_typed_requests() {
    use cortexkit_role_llm_runner::send::SendRequest;
    for key in [
        json!(""),
        json!("x".repeat(129)),
        json!("two words"),
        json!("tab\t"),
        json!("\u{0}"),
        json!("\u{7f}"),
        json!("é"),
        json!(null),
        json!(7),
        json!(true),
        json!([]),
        json!({}),
    ] {
        let value = json!({"key":key});
        for (result, field) in [
            (
                cache::check_plan(json!({"cache":value}).as_object().unwrap()).map(|_| ()),
                "plan.cache.key",
            ),
            (cache::check_send_value(&value).map(|_| ()), "cache.key"),
        ] {
            let error = result.expect_err("invalid key must not disappear during decoding");
            assert_eq!(error.code, "invalid_params");
            assert_eq!(error.detail, json!({"field":field}));
        }
        let error = serde_json::from_value::<SendRequest>(json!({
            "prompt":"hello", "send_id":"bad-key", "cache":value
        }))
        .unwrap_err();
        assert!(error.to_string().contains("cache.key"), "{error}");
        if let Some(key) = key.as_str() {
            let mut cache = Cache::new();
            cache.key = Some(key.to_owned());
            assert_refusal(&cache, "key");
        }
    }
}

#[test]
fn cache_midnight_crossing_is_valid_and_zone_recognition_is_the_runners() {
    let mut cache = Cache::new();
    cache.ttl = Some(Ttl::Long);
    cache.keep_warm = Some(KeepWarm::Hours);
    cache.hours = Some(Hours::new("22:00", "06:00", "Europe/Istanbul"));
    assert!(cache::check(&cache).is_ok());
    cache.hours.as_mut().unwrap().tz = "not/a/known-zone".into();
    assert!(
        cache::check(&cache).is_ok(),
        "zone databases belong to runners, not shape checks"
    );
    cache.hours = Some(Hours::new("00:00", "23:59", "Etc/UTC"));
    assert!(cache::check(&cache).is_ok());
}

#[test]
fn cache_send_lifecycle_and_both_locations_refuse_before_acceptance() {
    use cortexkit_role_llm_runner::send::SendRequest;
    let cache = request(json!({"key":"prefix-v1","ttl":"long","keep_warm":"always"}));
    let send = SendRequest::new("hello", "send-a").with_cache(cache);
    assert!(send.check_cache(true).is_ok());
    assert_eq!(
        send.check_cache(false).unwrap_err().detail,
        json!({"field":"cache"})
    );
    let encoded = serde_json::to_value(&send).unwrap();
    let decoded: SendRequest = serde_json::from_value(encoded.clone()).unwrap();
    assert_eq!(decoded.cache, send.cache);
    assert!(!decoded.runner_params.contains_key("cache"));
    assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
    let both = send.with_plan(serde_json::Map::new());
    assert_eq!(
        both.check_cache(true).unwrap_err().detail,
        json!({"field":"cache"})
    );
    let absent = SendRequest::new("hello", "send-b");
    assert!(absent.check_cache(false).is_ok());
    assert!(!serde_json::to_value(absent)
        .unwrap()
        .as_object()
        .unwrap()
        .contains_key("cache"));
}

#[test]
fn cache_vectors_cover_plan_send_and_step_report_round_trips() {
    use cortexkit_role_llm_runner::step_events::StepFinished;
    let vectors: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/cache-policy.json"
    ))
    .unwrap();
    for value in vectors["valid"].as_array().unwrap() {
        let cache = request(value.clone());
        assert!(cache::check(&cache).is_ok());
        assert_eq!(serde_json::to_value(cache).unwrap(), *value);
        let plan = json!({"cache":value});
        assert!(cache::check_plan(plan.as_object().unwrap())
            .unwrap()
            .is_some());
    }
    for case in vectors["invalid"].as_array().unwrap() {
        let policy = &case["cache"];
        for (result, prefix) in [
            (
                cache::check_plan(json!({"cache":policy}).as_object().unwrap()).map(|_| ()),
                "plan.cache",
            ),
            (cache::check_send_value(policy).map(|_| ()), "cache"),
        ] {
            let error = result.unwrap_err();
            assert_eq!(error.code, "invalid_params");
            assert_eq!(
                error.detail,
                json!({"field":format!("{prefix}.{}", case["field"].as_str().unwrap())})
            );
        }
    }
    let legacy: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/step-events.json"
    ))
    .unwrap();
    let event: StepFinished = serde_json::from_value(legacy["minimal"].clone()).unwrap();
    assert_eq!(event.cache, None);
    assert_eq!(serde_json::to_value(event).unwrap(), legacy["minimal"]);
    for report in vectors["reports"].as_array().unwrap() {
        let mut value = legacy["minimal"].clone();
        value["cache"] = report.clone();
        let event: StepFinished = serde_json::from_value(value.clone()).unwrap();
        assert!(event.check().is_ok());
        assert_eq!(serde_json::to_value(event).unwrap(), value);
    }
    let mut value = legacy["minimal"].clone();
    value["cache"] = json!({"ttl_ms":1000,"implicit":true});
    let event: StepFinished = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(event.check(), Err("cache.implicit"));
    value["cache"] = json!({"ttl_ms":1000,"warming":{"on":true,"stop_reason":"cost-cap"}});
    assert_eq!(
        serde_json::from_value::<StepFinished>(value.clone())
            .unwrap()
            .check(),
        Err("cache.warming.stop_reason")
    );
    value["cache"]["unexpected"] = json!(0);
    assert!(serde_json::from_value::<StepFinished>(value).is_err());
}
