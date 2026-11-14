use cortexkit_role_routing::{
    capabilities,
    describe::{check_describe, RoleDescribe},
    select::{Requirements, SelectAnswer, SelectRequest},
    select_media::*,
};
use serde_json::{json, Value};

fn request(requirements: Value) -> Value {
    json!({"consumer":"example.task","kind":"video","privacy":"remote_allowed","requirements":requirements})
}

fn parse(value: &Value) -> Result<SelectMediaRequest, cortexkit_role_routing::errors::Refusal> {
    parse_select_media_request(value, capabilities::KNOWN, &["video"])
}

#[test]
fn input_characters_is_an_unsigned_synthesize_only_requirement() {
    let mut value = request(json!({"input_characters":4,"input_bytes":9}));
    value["kind"] = json!("synthesize");
    let parse = |v: &Value| {
        parse_select_media_request(
            v,
            capabilities::KNOWN,
            &["synthesize", "video", "transcribe", "image", "realtime"],
        )
    };
    for count in [0, 4, u64::MAX] {
        value["requirements"]["input_characters"] = json!(count);
        let parsed = parse(&value).unwrap();
        assert_eq!(parsed.requirements.input_characters, Some(count));
        assert_eq!(serde_json::to_value(parsed).unwrap(), value);
    }
    for count in [json!(-1), json!(1.5), json!("4"), json!(false)] {
        value["requirements"]["input_characters"] = count;
        let refusal = parse(&value).unwrap_err();
        assert_eq!(
            refusal.code,
            cortexkit_role_routing::errors::RefusalCode::InvalidRequirements
        );
        assert_eq!(refusal.detail["field"], "input_characters");
    }
    value["requirements"]["input_characters"] = json!(4);
    for kind in ["video", "transcribe", "image", "realtime"] {
        value["kind"] = json!(kind);
        assert_eq!(
            parse(&value).unwrap_err().detail["field"],
            "input_characters"
        );
    }
    let llm = json!({"consumer":"example.task","requirements":{"iq":50,"input_characters":4}});
    assert_eq!(
        cortexkit_role_routing::select::parse_select_request(&llm)
            .unwrap_err()
            .detail["field"],
        "input_characters"
    );
    assert!(REQUIREMENT_FIELDS.contains(&"input_characters"));
}

#[test]
fn media_requirements_reuse_engine_types_and_round_trip_one_object() {
    let value = request(
        json!({"iq":0,"language":"en","voice":"alto","format":"video/mp4","duration_ms":1,"input_bytes":2,"reference_formats":["image/png"],"bias":false,"size":"large","resolution":"1080p","quality":"high","latency_class":"batch","metric":"score","output_duration_ms":3000,"aspect_ratio":"16:9","audio":true,"metric_limit":{"metric":"score","at_least":0.9}}),
    );
    let parsed = parse(&value).unwrap();
    assert_eq!(parsed.requirements.iq, Some(0));
    assert_eq!(parsed.requirements.engine.language.as_deref(), Some("en"));
    assert_eq!(serde_json::to_value(&parsed).unwrap(), value);
    assert_eq!(
        serde_json::from_value::<SelectMediaRequest>(value).unwrap(),
        parsed
    );
    assert_eq!(parse(&request(json!({}))).unwrap().requirements.iq, None);
}

#[test]
fn media_requirements_refuse_malformed_values_by_name() {
    for (field, value) in [
        ("iq", json!(101)),
        ("output_duration_ms", json!(-1)),
        ("audio", json!("yes")),
        ("aspect_ratio", json!("0:9")),
        ("aspect_ratio", json!("16:0")),
        ("aspect_ratio", json!("16:9:1")),
        ("aspect_ratio", json!("+16:9")),
        ("language", json!(true)),
        ("latency_class", json!("future")),
        ("unknown", json!(1)),
        ("metric_limit", json!({"metric":"wer"})),
        (
            "metric_limit",
            json!({"metric":"wer","at_most":0.1,"at_least":0.1}),
        ),
        ("metric_limit", json!({"metric":"wer","at_most":"NaN"})),
        (
            "metric_limit",
            json!({"metric":"wer","at_most":0.1,"extra":0}),
        ),
    ] {
        let refusal = parse(&request(json!({field:value}))).unwrap_err();
        assert_eq!(refusal.detail["field"], field);
    }
    for limit in [
        MetricLimit::at_least("score", 1.into()),
        MetricLimit::at_most("score", 1.into()),
    ] {
        limit.check().unwrap();
        assert!(limit.admits(Some(1.0)));
        assert!(!limit.admits(None));
        assert!(!limit.admits(Some(f64::NAN)));
    }
}

#[test]
fn media_requires_advertised_kind_privacy_and_valid_caller_local() {
    let value = request(json!({}));
    assert!(parse_select_media_request(&value, &["select_media"], &["video"]).is_err());
    assert!(parse_select_media_request(&value, &["select_kind"], &["video"]).is_err());
    assert_eq!(
        parse_select_media_request(&value, capabilities::KNOWN, &["image"])
            .unwrap_err()
            .detail["field"],
        "kind"
    );
    let mut value = value;
    value["privacy"] = json!("unknown");
    assert_eq!(parse(&value).unwrap_err().detail["field"], "privacy");
    value.as_object_mut().unwrap().remove("privacy");
    assert_eq!(parse(&value).unwrap_err().detail["field"], "privacy");
    value["privacy"] = json!("remote_allowed");
    value["caller_local"] = json!([{"kind":"video","provider":"example","model":"local","local":false,"formats":["video/mp4"],"latency_class":"batch"}]);
    assert_eq!(
        parse(&value).unwrap_err().detail["field"],
        "caller_local[0].local"
    );
    let describe = RoleDescribe::new(vec![], false).with_capability("select_media");
    assert!(check_describe(&describe).is_err());
    assert!(check_describe(&describe.with_capability("select_kind")).is_ok());
}

#[test]
fn media_answers_do_not_loosen_llm_request_or_answer() {
    assert!(serde_json::from_value::<SelectRequest>(
        json!({"consumer":"example.task","requirements":{}})
    )
    .is_err());
    assert_eq!(Requirements::iq(50).iq, 50);
    let value = json!({"decision_id":"decision","expires_at_ms":600000,"engine":{"provider":"example","model":"model","local":false,"caller_local":false,"module":"example-media"}});
    let _: SelectMediaAnswer = serde_json::from_value(value.clone()).unwrap();
    assert!(serde_json::from_value::<SelectAnswer>(value).is_err());
}

#[test]
fn media_unit_prices_are_verbatim_and_validate_without_token_rates() {
    let value = json!({"basis":"subscription_equivalent","multiplier":20,"rates":{"input.per_character":{"rate":0.1,"step":1000,"minimum":1},"input.per_audio_second":{"rate":2},"output.per_image":{"rate":3},"output.per_video_second":{"rate":4}},"estimated_task_usd":1});
    let price: UnitPrice = serde_json::from_value(value.clone()).unwrap();
    price.check().unwrap();
    assert_eq!(serde_json::to_value(&price).unwrap(), value);
    assert_eq!(
        price.rate("output.per_image").unwrap().rate,
        serde_json::Number::from(3)
    );
    assert!(price.rate("output.per_second").is_none());
    for bad in [
        json!({"basis":"metered","multiplier":2,"rates":{}}),
        json!({"basis":"subscription_equivalent","rates":{}}),
        json!({"basis":"metered","rates":{"input.per_character":{"rate":-1}}}),
        json!({"basis":"metered","rates":{"input.per_character":{"rate":1,"step":0}}}),
    ] {
        assert!(serde_json::from_value::<UnitPrice>(bad)
            .unwrap()
            .check()
            .is_err());
    }
}
