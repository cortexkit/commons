use std::collections::BTreeMap;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use cortexkit_role_media::{
    bias::*, describe::*, errors::*, jobs::*, objects::*, realtime::*, streams::*, usage::*,
    vectors, *,
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

fn vector(name: &str) -> Value {
    serde_json::from_str(vectors::file(name).unwrap()).unwrap()
}
fn decode<T: DeserializeOwned>(value: &Value) -> T {
    serde_json::from_value(value.clone()).unwrap()
}
fn round_trip<T: DeserializeOwned + Serialize>(value: &Value) -> T {
    let typed: T = decode(value);
    assert_eq!(serde_json::to_value(&typed).unwrap(), *value);
    typed
}

fn describe() -> Value {
    json!({"groups":["jobs","usage_feed"],"jobs_survive_restart":false,"engines":[{
        "kind":"transcribe","provider":"local","model":"speech","local":true,
        "formats":["audio/wav"],"latency_class":"batch"}]})
}
fn describe_code(value: Value, expected: &str) {
    assert_eq!(check_describe_value(&value).unwrap_err().code, expected);
}

#[test]
fn media_contract_describe_valid_and_builders() {
    let typed: Describe = round_trip(&describe());
    check_describe(&typed).unwrap();
    let engine = Engine::new(
        Kind::Transcribe,
        "local".into(),
        "speech".into(),
        true,
        vec!["audio/wav".into()],
        LatencyClass::Batch,
    )
    .with_bias_limit(BiasLimit::new(BiasUnit::Phrase, 3));
    check_describe(
        &Describe::new(vec![Group::Jobs], vec![engine]).with_jobs_survive_restart(false),
    )
    .unwrap();
}
#[test]
fn media_contract_describe_usage_without_usage_feed() {
    let mut d = describe();
    d["groups"] = json!(["jobs"]);
    d["engines"][0]["local"] = json!(false);
    describe_code(d, "usage_without_usage_feed");
}
#[test]
fn media_contract_describe_stream_without_jobs() {
    let mut d = describe();
    d["groups"] = json!(["stream_transcribe"]);
    describe_code(d, "stream_without_jobs");
}
#[test]
fn media_contract_describe_voice_front_without_realtime() {
    let mut d = describe();
    d["groups"] = json!(["voice-front/v1"]);
    describe_code(d, "voice_front_without_realtime");
}
#[test]
fn media_contract_describe_engine_without_group() {
    let mut d = describe();
    d["groups"] = json!([]);
    describe_code(d, "engine_without_group");
}
#[test]
fn media_contract_describe_group_fact_invalid() {
    for invalid in [Value::Null, json!(1), json!("false")] {
        let mut d = describe();
        d["jobs_survive_restart"] = invalid;
        let error = check_describe_value(&d).unwrap_err();
        assert_eq!(error.code, "group_fact_invalid");
        assert_eq!(error.field.as_deref(), Some("jobs_survive_restart"));
    }
    for fact in ["token_max_ttl_ms", "call_silence_timeout_s"] {
        for invalid in [Value::Null, json!(0), json!(-1), json!(1.5), json!(true)] {
            let mut d = describe();
            d["groups"] = json!(["jobs", "usage_feed", "realtime"]);
            d["token_max_ttl_ms"] = json!(1);
            d["call_silence_timeout_s"] = json!(1);
            d[fact] = invalid;
            let error = check_describe_value(&d).unwrap_err();
            assert_eq!(error.code, "group_fact_invalid");
            assert_eq!(error.field.as_deref(), Some(fact));
        }
    }
}
#[test]
fn media_contract_describe_same_kind_model_different_providers() {
    let cases = vector("describe.json");
    let d = &cases[0]["describe"];
    check_describe_value(d).unwrap();
    let typed: Describe = round_trip(d);
    check_describe(&typed).unwrap();
}
#[test]
fn media_contract_describe_duplicate_engine() {
    let mut d = describe();
    let engine = d["engines"][0].clone();
    d["engines"].as_array_mut().unwrap().push(engine);
    describe_code(d, "duplicate_engine");
    let cases = vector("describe.json");
    describe_code(cases[1]["describe"].clone(), "duplicate_engine");
}
#[test]
fn media_contract_describe_fact_too_long() {
    for fact in [
        "provider",
        "model",
        "formats",
        "reference_formats",
        "languages",
        "voices",
        "sizes",
        "resolutions",
        "qualities",
    ] {
        let mut d = describe();
        let text = "😀".repeat(257);
        d["engines"][0][fact] = if matches!(fact, "provider" | "model") {
            json!(text)
        } else {
            json!([text])
        };
        describe_code(d, "fact_too_long");
    }
}
#[test]
fn media_contract_describe_bias_limit_invalid() {
    for max in [json!(0), json!(-1)] {
        let mut d = describe();
        d["engines"][0]["bias_limit"] = json!({"unit":"phrase","max":max});
        describe_code(d, "bias_limit_invalid");
    }
}
#[test]
fn media_contract_describe_formats_empty() {
    let mut d = describe();
    d["engines"][0]["formats"] = json!([]);
    describe_code(d, "formats_empty");
}
#[test]
fn media_contract_describe_local_realtime_engine() {
    let mut d = describe();
    d["groups"] = json!(["jobs", "usage_feed", "realtime"]);
    d["token_max_ttl_ms"] = json!(1);
    d["call_silence_timeout_s"] = json!(1);
    d["engines"][0]["kind"] = json!("realtime");
    describe_code(d, "local_realtime_engine");
}
#[test]
fn media_contract_describe_describe_too_large() {
    let mut d = describe();
    d["padding"] = json!("a".repeat(MESSAGE_MAX));
    describe_code(d, "describe_too_large");
}

#[test]
fn media_contract_all_open_enums_round_trip_unknown() {
    macro_rules! unknown {
        ($($ty:ident),+ $(,)?) => {$(
            let value: $ty = round_trip(&json!("future_value"));
            assert!(matches!(value, $ty::Unknown(_)));
        )+};
    }
    unknown!(
        Group,
        Kind,
        LatencyClass,
        BiasUnit,
        JobStatus,
        RefusalClass,
        ChargeBasis,
        AuthMethod,
        Unit,
        MeasurementRole,
        WatchType
    );
    for value in vector("refusals.json").as_array().unwrap() {
        let refusal: Refusal = round_trip(value);
        let expected = value["detail"]["class"] == "transient";
        assert_eq!(refusal.detail.class.is_retryable(), expected);
    }
    assert!(!RefusalClass::Unknown("future_class".into()).is_retryable());
    assert!(serde_json::from_value::<UsageStatus>(json!("future")).is_err());
    assert!(serde_json::from_value::<Measure>(json!("future")).is_err());
    assert!(serde_json::from_value::<Source>(json!("future")).is_err());
}

#[test]
fn media_contract_terminal_failure_bounds_and_routing_detail() {
    let failure = Failure::new(
        "routing_refused".into(),
        RefusalClass::Transient,
        "😀".repeat(1024),
    )
    .with_detail(BTreeMap::from([(
        "routing_code".into(),
        "account_hold".into(),
    )]));
    check_failure(&failure).unwrap();
    let mut broken = failure.clone();
    broken.message.push('😀');
    assert!(check_failure(&broken).is_err());
    broken = failure.clone();
    broken.detail = None;
    assert!(check_failure(&broken).is_err());
    broken = failure.clone();
    broken
        .detail
        .as_mut()
        .unwrap()
        .insert("routing_code".into(), "😀".repeat(257));
    assert!(check_failure(&broken).is_err());
    assert_eq!(
        truncate_failure_message(&"😀".repeat(1025)),
        "😀".repeat(1024)
    );
    for code in [
        "account_hold",
        "account_pool_exhausted",
        "account_selection_unavailable",
    ] {
        assert!(routing_refusal_class(code).is_retryable());
    }
    assert!(!routing_refusal_class("unknown_decision").is_retryable());
    let status = StatusAnswer::new(JobStatus::Failed).with_failure(failure.clone());
    check_status(&status).unwrap();
    assert_eq!(
        serde_json::to_value(status).unwrap()["failure"],
        serde_json::to_value(failure).unwrap()
    );
}

#[test]
fn media_contract_kind_vectors() {
    for case in vector("kinds.json").as_array().unwrap() {
        if !case["output_object"].is_null() {
            let value: OutputObject = round_trip(&case["output_object"]);
            check_media(&value.output_object).unwrap();
            continue;
        }
        let result = match case["kind"].as_str().unwrap() {
            "transcribe" => {
                round_trip::<TranscribeInput>(&case["input"]);
                check_transcribe_output(&round_trip(&case["output"]))
            }
            "synthesize" => {
                round_trip::<SynthesizeInput>(&case["input"]);
                check_synthesize_output(&round_trip(&case["output"]))
            }
            "image" => {
                round_trip::<ImageInput>(&case["input"]);
                check_image_output(&round_trip(&case["output"]))
            }
            "video" => {
                round_trip::<VideoInput>(&case["input"]);
                check_video_output(&round_trip(&case["output"]))
            }
            _ => panic!("unknown vector kind"),
        };
        assert_eq!(
            result.is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn media_contract_bias_trim_vectors() {
    for case in vector("trim-bias.json").as_array().unwrap() {
        let costs: Vec<u64> = decode(&case["costs"]);
        let actual = trim_bias(
            &costs,
            case["prompt_cost"].as_u64(),
            case["max"].as_u64().unwrap(),
        );
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            case["expected"],
            "{}",
            case["name"]
        );
    }
}

fn handoff_mismatches(implementation: impl Fn(&str, &[String]) -> String) -> Vec<String> {
    vector("hand-off.json")["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|case| {
            let turns: Vec<String> = decode(&case["turns"]);
            (implementation(case["framing"].as_str().unwrap(), &turns)
                != case["expected"].as_str().unwrap())
            .then(|| case["name"].as_str().unwrap().to_owned())
        })
        .collect()
}
#[test]
fn media_contract_hand_off_message_vectors_and_faulty_implementation() {
    assert!(handoff_mismatches(hand_off_message).is_empty());
    // A faulty producer inserts transcript text without the data-quoting prefix.
    fn broken(framing: &str, turns: &[String]) -> String {
        format!("[voice front: {framing}]\n{}", turns.join("\n"))
    }
    let failures = handoff_mismatches(broken);
    assert!(failures.contains(&"all_breaks_and_quote_prefixes".to_string()));
}
#[test]
fn media_contract_hand_off_send_id_vectors() {
    for case in vector("hand-off.json")["send_ids"].as_array().unwrap() {
        assert_eq!(
            hand_off_send_id(
                case["call_id"].as_str().unwrap(),
                case["principal"].as_str().unwrap(),
                case["n"].as_u64().unwrap()
            ),
            case["expected"]
        );
    }
}

#[test]
fn media_contract_packs_and_privacy_fail_closed() {
    for case in vector("packs.json").as_array().unwrap() {
        round_trip::<ContextRequest>(&case["request"]);
        let pack: ContextAnswer = round_trip(&case["answer"]);
        check_pack(&pack).unwrap();
    }
    let privacy: Privacy = BTreeMap::new();
    for kind in [
        Kind::Transcribe,
        Kind::Synthesize,
        Kind::Image,
        Kind::Video,
        Kind::StreamTranscribe,
        Kind::Realtime,
    ] {
        assert_eq!(
            privacy_for(&privacy, &kind),
            PrivacyRule::new(MediaPrivacy::LocalOnly, false)
        );
        assert!(check_privacy(&privacy, &kind, true, false).is_err());
        check_privacy(&privacy, &kind, false, true).unwrap();
    }
    let privacy = BTreeMap::from([(
        Kind::Transcribe,
        PrivacyRule::new(MediaPrivacy::RemoteAllowed, false),
    )]);
    assert!(check_privacy(&privacy, &Kind::Transcribe, true, true).is_err());
    check_privacy(&privacy, &Kind::Transcribe, true, false).unwrap();
    assert!(serde_json::from_value::<ContextAnswer>(json!({"version":"v","persona":1,"memories":[],"board":{},"recent":{},"built_at_ms":0,"privacy":{}})).is_err());
    assert!(serde_json::from_value::<ContextAnswer>(json!({"version":"v","terms":[],"built_at_ms":0,"privacy":{"transcribe":{"media":"unsafe","bias_remote":true}}})).is_err());
    let front = FrontPack::new(
        "v".into(),
        "helpful".into(),
        vec![],
        json!({"unknown":[1,true]}),
        Value::Null,
        0,
        BTreeMap::new(),
    );
    check_pack(&ContextAnswer::Front(front.clone())).unwrap();
    let mut invalid = front.clone();
    invalid.memories.push(json!(1));
    assert!(check_pack(&ContextAnswer::Front(invalid)).is_err());
    let mut invalid = front;
    invalid.persona = "a".repeat(MESSAGE_MAX);
    assert!(check_pack(&ContextAnswer::Front(invalid)).is_err());
    for term in [
        BiasTerm::new("".into(), 0.5),
        BiasTerm::new("a".repeat(257), 0.5),
        BiasTerm::new("a".into(), 1.1),
    ] {
        assert!(check_bias(&Bias::new(vec![term])).is_err());
    }
}

#[test]
fn media_contract_chunk_vectors_and_object_integrity() {
    let cases = vector("chunks.json");
    for case in cases["uploads"].as_array().unwrap() {
        let bytes = vec![
            case["byte"].as_u64().unwrap() as u8;
            case["decoded_bytes"].as_u64().unwrap() as usize
        ];
        let data = STANDARD.encode(&bytes);
        let request = UploadAppendRequest::new("upload-1".into(), 0, data.clone());
        assert_eq!(
            check_upload_append(&request, 0).is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
        let media = MediaValue::new(
            "audio/wav".into(),
            bytes.len() as u64,
            format!("{:x}", Sha256::digest(&bytes)),
        )
        .with_data(data);
        assert_eq!(
            check_media(&media).is_ok(),
            case["valid"].as_bool().unwrap()
        );
    }
    for case in cases["reads"].as_array().unwrap() {
        assert_eq!(
            check_read(
                &ReadRequest::new("object-1".into(), 0, case["length"].as_u64().unwrap()),
                CHUNK_MAX
            )
            .is_ok(),
            case["valid"].as_bool().unwrap()
        );
    }
    let lifecycle = &cases["lifecycle"];
    round_trip::<UploadBeginRequest>(&lifecycle["begin"]);
    round_trip::<UploadBeginAnswer>(&lifecycle["begun"]);
    let append: UploadAppendRequest = round_trip(&lifecycle["append"]);
    check_upload_append(&append, 0).unwrap();
    assert_eq!(
        check_upload_append(&append, 1).unwrap_err().code,
        "upload_offset_mismatch"
    );
    round_trip::<UploadAppendAnswer>(&lifecycle["appended"]);
    round_trip::<UploadStatusAnswer>(&lifecycle["status"]);
    let committed: UploadCommitAnswer = round_trip(&lifecycle["committed"]);
    round_trip::<ReadRequest>(&lifecycle["read"]);
    round_trip::<ReadAnswer>(&lifecycle["read_answer"]);
    verify_object(b"a", committed.bytes, &committed.sha256).unwrap();
    assert!(verify_object(b"b", committed.bytes, &committed.sha256).is_err());
    assert!(verify_object(b"aa", committed.bytes, &committed.sha256).is_err());
    let media = MediaValue::new(committed.mime, committed.bytes, committed.sha256)
        .with_object_id("upload-1".into());
    check_media(&media).unwrap();
    let mut mismatch = media.clone();
    mismatch.bytes += 1;
    assert_eq!(
        check_object_facts(&mismatch, &media, "input.audio")
            .unwrap_err()
            .code,
        "object_mismatch"
    );
    let mut both = media.clone();
    both.data = Some("YQ==".into());
    assert!(check_media(&both).is_err());
    let mut neither = media;
    neither.object_id = None;
    assert!(check_media(&neither).is_err());
    check_read(&ReadRequest::new("object-1".into(), 1, 1), 1).unwrap();
    assert!(check_read(&ReadRequest::new("object-1".into(), 2, 1), 1).is_err());
    assert!(check_read(&ReadRequest::new("object-1".into(), 0, 0), 1).is_err());
}

#[test]
fn media_contract_message_budget_vectors() {
    for case in vector("budgets.json").as_array().unwrap() {
        let input = if let Some(sizes) = case["inline_bytes"].as_array() {
            let references: Vec<MediaValue> = sizes
                .iter()
                .map(|size| {
                    let bytes = vec![97; size.as_u64().unwrap() as usize];
                    let media = MediaValue::new(
                        "image/png".into(),
                        bytes.len() as u64,
                        format!("{:x}", Sha256::digest(&bytes)),
                    )
                    .with_data(STANDARD.encode(bytes));
                    check_media(&media).unwrap();
                    media
                })
                .collect();
            json!({"prompt":"a tree","size":"512x512","references":references})
        } else {
            json!({"text":"a".repeat(case["text_bytes"].as_u64().unwrap() as usize),"voice":"calm","format":"audio/wav"})
        };
        let params =
            json!({"client_job_id":"job-1","kind":case["kind"],"model":"example","input":input});
        assert!(serde_json::to_vec(&params).unwrap().len() > MESSAGE_MAX);
        let error = check_message_budget(&params).unwrap_err();
        assert_eq!(error.code, "invalid_params");
        assert_eq!(error.field.as_deref(), case["field"].as_str());
    }
    // A JSON string's two quotes count against the encoded message budget.
    check_message_budget(&json!("a".repeat(MESSAGE_MAX - 2))).unwrap();
    assert!(check_message_budget(&json!("a".repeat(MESSAGE_MAX - 1))).is_err());
}

fn basic_record() -> UsageRecord {
    decode(&vector("usage.json")["records"][0])
}

fn request_derived_record() -> UsageRecord {
    basic_record().with_measurements(vec![Measurement::new(
        MeasurementRole::Input,
        Unit::Character,
        "12".into(),
        Measure::Billed,
        Source::RequestDerived,
    )])
}

#[test]
fn media_contract_usage_request_derived_round_trip() {
    let source: Source = round_trip(&json!("request_derived"));
    assert_eq!(source, Source::RequestDerived);
    let value = vector("usage.json")["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|value| value["record_id"] == "r-request-derived")
        .unwrap()
        .clone();
    let record: UsageRecord = round_trip(&value);
    assert_eq!(record.measurements[0].source, Source::RequestDerived);
    assert_eq!(record.measurements[0].measure, Measure::Billed);
    check_usage_record(&record).unwrap();
}

#[test]
fn media_contract_usage_request_derived_refuses_non_done_billing() {
    for status in [
        UsageStatus::Failed,
        UsageStatus::Cancelled,
        UsageStatus::Ended,
    ] {
        let mut record = request_derived_record();
        record.status = status;
        if status == UsageStatus::Failed {
            record.failure = Some(Failure::new(
                "engine_crashed".into(),
                RefusalClass::Permanent,
                "Failed request".into(),
            ));
        }
        let error = check_usage_record(&record).expect_err("non-done request-derived billing");
        assert_eq!(error.code, "invalid_usage_record", "{status:?}");
        assert_eq!(error.field.as_deref(), Some("status"), "{status:?}");
        record.measurements[0].measure = Measure::Observed;
        check_usage_record(&record).unwrap();
    }
    let mut record = request_derived_record();
    record.measurements[0].measure = Measure::Observed;
    check_usage_record(&record).unwrap();
}

#[test]
fn media_contract_usage_request_derived_refuses_vendor_reply_billing() {
    let mut record = request_derived_record();
    let mut vendor = record.measurements[0].clone();
    vendor.source = Source::VendorReply;
    record.measurements.push(vendor);
    for _ in 0..2 {
        let error = check_usage_record(&record).expect_err("duplicate billed role and unit");
        assert_eq!(error.code, "invalid_usage_record");
        assert_eq!(error.field.as_deref(), Some("measurements"));
        record.measurements.reverse();
    }
    let valid = record.clone();
    record.measurements[1].measure = Measure::Observed;
    check_usage_record(&record).unwrap();
    record = valid.clone();
    record.measurements[0].measure = Measure::Observed;
    check_usage_record(&record).unwrap();
    record = valid.clone();
    record.measurements[1].role = MeasurementRole::Output;
    check_usage_record(&record).unwrap();
    record = valid;
    record.measurements[1].unit = Unit::Token;
    check_usage_record(&record).unwrap();
}

fn usage_role_round_trip(role: &str) -> MeasurementRole {
    let mut value = serde_json::to_value(basic_record()).unwrap();
    value["measurements"] = json!([{
        "role": role,
        "unit": "token",
        "quantity": "123.00",
        "measure": "billed",
        "source": "vendor_reply"
    }]);
    let record: UsageRecord = round_trip(&value);
    check_usage_record(&record).unwrap();
    assert_eq!(record.measurements[0].unit, Unit::Token);
    assert_eq!(record.measurements[0].role.as_str(), role);
    record.measurements.into_iter().next().unwrap().role
}

#[test]
fn media_contract_usage_input_image_round_trip() {
    assert!(!matches!(
        usage_role_round_trip("input_image"),
        MeasurementRole::Unknown(_)
    ));
}

#[test]
fn media_contract_usage_output_image_round_trip() {
    assert!(!matches!(
        usage_role_round_trip("output_image"),
        MeasurementRole::Unknown(_)
    ));
}

#[test]
fn media_contract_usage_output_video_round_trip() {
    assert!(!matches!(
        usage_role_round_trip("output_video"),
        MeasurementRole::Unknown(_)
    ));
}

#[test]
fn media_contract_usage_cache_read_round_trip() {
    assert!(!matches!(
        usage_role_round_trip("cache_read"),
        MeasurementRole::Unknown(_)
    ));
}

#[test]
fn media_contract_usage_unknown_role_round_trip() {
    assert_eq!(
        usage_role_round_trip("future_usage_role"),
        MeasurementRole::Unknown("future_usage_role".into())
    );
}

#[test]
fn media_contract_usage_vectors_exact_decimals_and_statuses() {
    let cases = vector("usage.json");
    for value in cases["records"].as_array().unwrap() {
        let record: UsageRecord = round_trip(value);
        check_usage_record(&record).unwrap();
    }
    for valid in cases["valid_quantities"].as_array().unwrap() {
        assert!(decimal_is_valid(valid.as_str().unwrap()), "{valid}");
    }
    for invalid in cases["invalid_quantities"].as_array().unwrap() {
        assert!(!decimal_is_valid(invalid.as_str().unwrap()), "{invalid}");
    }
    assert_eq!(
        bounded_vendor_request_id(&"v".repeat(257)),
        cases["vendor_request_id"]["expected"]
    );
    assert_eq!(
        bounded_vendor_request_id(&"😀".repeat(256)),
        "😀".repeat(256)
    );
    let mut record = basic_record();
    record.stream_id = Some("stream-1".into());
    assert!(check_usage_record(&record).is_err());
    record = basic_record();
    record.job_id = None;
    assert!(check_usage_record(&record).is_err());
    record = basic_record();
    record.status = UsageStatus::Failed;
    assert!(check_usage_record(&record).is_err());
    record = basic_record();
    record.failure = Some(Failure::new(
        "error".into(),
        RefusalClass::Permanent,
        "bad".into(),
    ));
    assert!(check_usage_record(&record).is_err());
    record = basic_record();
    record.charge_basis = ChargeBasis::Local;
    assert!(check_usage_record(&record).is_err());
    let mut value = serde_json::to_value(basic_record()).unwrap();
    value.as_object_mut().unwrap().remove("principal");
    assert!(serde_json::from_value::<UsageRecord>(value).is_err());
    let mut value = serde_json::to_value(basic_record()).unwrap();
    value["measurements"][0]["quantity"] = json!(9007199254740993_u64);
    assert!(serde_json::from_value::<UsageRecord>(value).is_err());
}

fn refused_usage_sample(id: &str) {
    let cases = vector("usage.json");
    let case = cases["refused_records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["record"]["record_id"] == id)
        .unwrap();
    let record: UsageRecord = round_trip(&case["record"]);
    let error = check_usage_record(&record).expect_err(id);
    assert_eq!(error.code, case["code"].as_str().unwrap(), "{id}");
    assert_eq!(error.field.as_deref(), case["field"].as_str(), "{id}");
}

#[test]
fn media_contract_usage_refuses_r_done() {
    refused_usage_sample("r-done");
}

#[test]
fn media_contract_usage_refuses_r_ended() {
    refused_usage_sample("r-ended");
}

#[test]
fn media_contract_usage_refuses_ten_fractional_digits() {
    refused_usage_sample("r-fraction-over");
}

#[test]
fn media_contract_usage_refuses_whole_above_i64_max() {
    refused_usage_sample("r-whole-over");
}

#[test]
fn media_contract_usage_refuses_coefficient_overflow_at_nine_decimals() {
    refused_usage_sample("r-coefficient-over");
}

#[test]
fn media_contract_usage_refuses_coefficient_overflow_at_whole_max() {
    refused_usage_sample("r-whole-fraction-edge");
}

#[test]
fn media_contract_usage_unicode_string_and_entry_bounds() {
    let recipe = vector("usage.json")["bounds"].clone();
    let text = recipe["scalar"]
        .as_str()
        .unwrap()
        .repeat(recipe["scalar_limit"].as_u64().unwrap() as usize);
    let measurement = Measurement::new(
        MeasurementRole::Unknown(text.clone()),
        Unit::Unknown(text.clone()),
        "9223372036.854775807".into(),
        Measure::Billed,
        Source::VendorReply,
    );
    let selectors: BTreeMap<String, String> = (1..=16)
        .map(|n| {
            (
                format!(
                    "{}{}",
                    char::from_u32(0x1f600 + n).unwrap(),
                    "😀".repeat(255)
                ),
                text.clone(),
            )
        })
        .collect();
    let failure = Failure::new(
        text.clone(),
        RefusalClass::Unknown(text.clone()),
        "😀".repeat(1024),
    )
    .with_detail(BTreeMap::from([
        ("field".into(), text.clone()),
        ("limit".into(), text.clone()),
    ]));
    let record = UsageRecord::new(
        text.clone(),
        text.clone(),
        text.clone(),
        text.clone(),
        ChargeBasis::Unknown(text.clone()),
        UsageStatus::Failed,
    )
    .with_job_id(text.clone())
    .with_decision_id(text.clone())
    .with_credential_ref(text.clone())
    .with_auth_method(AuthMethod::Unknown(text.clone()))
    .with_vendor_request_id(text.clone())
    .with_failure(failure)
    .with_selectors(selectors)
    .with_measurements(vec![measurement; 16]);
    check_usage_record(&record).unwrap();
    let value = serde_json::to_value(&record).unwrap();
    fn string_paths(value: &Value, path: String, out: &mut Vec<String>) {
        match value {
            Value::String(_) => out.push(path),
            Value::Object(map) => {
                for (key, value) in map {
                    string_paths(value, format!("{path}/{key}"), out);
                }
            }
            Value::Array(values) => {
                for (index, value) in values.iter().enumerate() {
                    string_paths(value, format!("{path}/{index}"), out);
                }
            }
            _ => {}
        }
    }
    let mut paths = Vec::new();
    string_paths(&value, String::new(), &mut paths);
    for path in paths {
        // Closed enums and fixed detail keys have fixed vocabulary, not a free string limit.
        let current = value.pointer(&path).unwrap().as_str().unwrap();
        if current.chars().count() < 256 {
            continue;
        }
        let mut broken = value.clone();
        *broken.pointer_mut(&path).unwrap() = json!(format!("{current}😀"));
        let broken: UsageRecord = decode(&broken);
        assert!(check_usage_record(&broken).is_err(), "{path}");
    }
    let mut broken = record.clone();
    broken.measurements.push(record.measurements[0].clone());
    assert!(check_usage_record(&broken).is_err());
    let mut broken = record.clone();
    broken.selectors.insert("extra".into(), "extra".into());
    assert!(check_usage_record(&broken).is_err());
    let mut broken = record;
    let key = broken.selectors.keys().next().unwrap().clone();
    let value = broken.selectors.remove(&key).unwrap();
    broken.selectors.insert(format!("{key}😀"), value);
    assert!(check_usage_record(&broken).is_err());
}

#[test]
fn media_contract_usage_encoded_size_boundary() {
    let recipe = vector("usage.json")["size"].clone();
    let mut record = basic_record();
    record.status = UsageStatus::Failed;
    record.failure = Some(
        Failure::new(
            "provider_failure".into(),
            RefusalClass::Permanent,
            "bad".into(),
        )
        .with_detail(
            (0..recipe["detail_members"].as_u64().unwrap())
                .map(|n| {
                    (
                        format!("detail_{n}"),
                        "a".repeat(recipe["member_bytes"].as_u64().unwrap() as usize),
                    )
                })
                .collect(),
        ),
    );
    // Many bounded members can still cross the byte budget; trim the final
    // members to hit each exact encoded length without oversized strings.
    for target in recipe["encoded_sizes"].as_array().unwrap() {
        let target = target.as_u64().unwrap() as usize;
        let mut candidate = record.clone();
        loop {
            let size = serde_json::to_vec(&candidate).unwrap().len();
            if size <= target {
                break;
            }
            let detail = candidate.failure.as_mut().unwrap().detail.as_mut().unwrap();
            let key = detail.keys().next_back().unwrap().clone();
            let remove = (size - target).min(detail[&key].len());
            if remove == 0 {
                detail.remove(&key);
            } else {
                let text = detail.get_mut(&key).unwrap();
                text.truncate(text.len() - remove);
            }
        }
        let size = serde_json::to_vec(&candidate).unwrap().len();
        if size < target {
            candidate
                .failure
                .as_mut()
                .unwrap()
                .message
                .push_str(&"a".repeat(target - size));
        }
        assert_eq!(serde_json::to_vec(&candidate).unwrap().len(), target);
        assert_eq!(
            check_usage_record(&candidate).is_ok(),
            target < USAGE_RECORD_MAX
        );
    }
}

#[test]
fn media_contract_usage_feed_entries_ack_answers_and_bounds() {
    let cases = vector("usage-feed.json");
    for case in cases["entries"].as_array().unwrap() {
        let entry: UsageEntry = round_trip(&case["entry"]);
        assert_eq!(
            check_usage_entry(&entry).is_ok(),
            case["valid"].as_bool().unwrap()
        );
    }
    for case in cases["acks"].as_array().unwrap() {
        let request: UsageAckRequest = round_trip(&case["request"]);
        let answer = usage_ack(
            request.through_seq,
            case["watermark"].as_u64().unwrap(),
            case["highest_written"].as_u64().unwrap(),
        )
        .unwrap();
        assert_eq!(serde_json::to_value(answer).unwrap(), case["answer"]);
    }
    assert!(usage_ack(1, 0, 0).is_err());
    for limit in [1, 1000] {
        check_usage_read(&UsageReadRequest::new(limit)).unwrap();
    }
    for limit in [0, 1001] {
        assert!(check_usage_read(&UsageReadRequest::new(limit)).is_err());
    }
    let raw = UsageEntry::new(1).with_raw(RawEntry::new("/w==".into(), "undecodable".into()));
    check_usage_answer(&UsageReadAnswer::new(vec![raw.clone()]).with_last_seq(1)).unwrap();
    assert!(check_usage_answer(&UsageReadAnswer::new(vec![raw.clone()])).is_err());
    assert!(
        check_usage_answer(&UsageReadAnswer::new(vec![raw.clone(), raw]).with_last_seq(1)).is_err()
    );
    check_usage_answer(&UsageReadAnswer::new(vec![])).unwrap();
    let mut bad: UsageEntry = decode(&cases["entries"][2]["entry"]);
    bad.raw.as_mut().unwrap().bytes_b64 = "YQ==".into();
    assert!(check_usage_entry(&bad).is_err());
    bad = decode(&cases["entries"][2]["entry"]);
    bad.raw.as_mut().unwrap().bytes = None;
    assert!(check_usage_entry(&bad).is_err());
}

#[test]
fn media_contract_realtime_stream_vectors_and_redacted_debug() {
    let v = vector("realtime.json");
    round_trip::<TokenRequest>(&v["token_request"]);
    let token: TokenAnswer = round_trip(&v["token_answer"]);
    assert!(!format!("{token:?}").contains(&token.token));
    round_trip::<CallStartedRequest>(&v["call_started"]);
    round_trip::<CallStartedAnswer>(&v["started_answer"]);
    round_trip::<CallEndedRequest>(&v["call_ended"]);
    round_trip::<CallEndedAnswer>(&v["ended_answer"]);
    round_trip::<WatchRequest>(&v["watch"]);
    round_trip::<WatchEvent>(&v["event"]);
    round_trip::<StreamRequest>(&v["stream_request"]);
    round_trip::<AudioFrame>(&v["audio_frame"]);
    round_trip::<EndFrame>(&v["end_frame"]);
    round_trip::<StreamEvent>(&v["partial"]);
    round_trip::<StreamEvent>(&v["final"]);
    let answer: StreamAnswer = round_trip(&v["stream_answer"]);
    check_stream_answer(&answer).unwrap();
    let answer: StreamAnswer = round_trip(&v["stream_failure"]);
    check_stream_answer(&answer).unwrap();
    assert!(
        check_stream_answer(&StreamAnswer::Success(StreamSuccess::new(vec![], false))).is_err()
    );
    check_stream_answer(&StreamAnswer::Success(StreamSuccess::new(vec![], true))).unwrap();
}

#[test]
fn media_contract_live_call_tool_values_round_trip() {
    let v = vector("realtime.json");
    for value in v["tool_events"].as_array().unwrap() {
        let event: WatchEvent = round_trip(value);
        check_watch_event(&event).unwrap();
        assert_eq!(
            event.tool.unwrap().as_str(),
            value["tool"].as_str().unwrap()
        );
    }
    assert_eq!(WatchTool::new("ask"), WatchTool::Ask);
    assert_eq!(WatchTool::new("hand_off"), WatchTool::HandOff);
    assert_eq!(
        WatchTool::new("future_tool"),
        WatchTool::Unknown("future_tool".into())
    );
}

#[test]
fn media_contract_live_call_watch_without_tool_keeps_old_bytes() {
    let bytes = r#"{"type":"tool_call","call_id":"call-1","at_ms":1}"#;
    let event: WatchEvent = serde_json::from_str(bytes).unwrap();
    assert_eq!(serde_json::to_string(&event).unwrap(), bytes);
    let mut value: Value = serde_json::from_str(bytes).unwrap();
    value["future_member"] = json!(true);
    let event: WatchEvent = serde_json::from_value(value).unwrap();
    assert_eq!(serde_json::to_string(&event).unwrap(), bytes);
}

#[test]
fn media_contract_live_call_refusal_reasons() {
    let refusals = vector("refusals.json");
    for reason in ["fork_unsupported", "fork_refused", "future_reason"] {
        let value = refusals
            .as_array()
            .unwrap()
            .iter()
            .find(|value| {
                value["code"] == "front_unavailable" && value["detail"]["reason"] == reason
            })
            .expect("front_unavailable reason vector missing");
        let refusal: Refusal = round_trip(value);
        assert_eq!(refusal.detail.members["reason"], reason);
        assert!(format!("{refusal:?}").contains(reason));
        assert!(!refusal.detail.class.is_retryable());
    }
}

#[test]
fn media_contract_live_call_operation_names_are_pinned() {
    let contract = include_str!("../CONTRACT.md");
    assert!(contract.contains("`media.stream.frames`: `media.stream_transcribe`"));
}

#[test]
fn media_contract_live_call_watch_operation_name_is_pinned() {
    let contract = include_str!("../CONTRACT.md");
    assert!(contract.contains("`media.realtime.watch`: `realtime.watch`"));
}

#[test]
fn media_contract_live_call_tool_only_on_tool_call() {
    for event_type in [
        WatchType::Started,
        WatchType::Ended,
        WatchType::new("future_type"),
    ] {
        let event = WatchEvent::new(event_type, "call-1".into(), 1).with_tool(WatchTool::Ask);
        let error = check_watch_event(&event).unwrap_err();
        assert_eq!(error.code, "invalid_params");
        assert_eq!(error.field.as_deref(), Some("tool"));
    }
}

#[test]
fn media_contract_requests_reject_unknown_members_and_ids_count_scalars() {
    macro_rules! closed {
        ($ty:ty, $value:expr) => {{
            let mut value: Value = $value;
            let _: $ty = decode(&value);
            value
                .as_object_mut()
                .unwrap()
                .insert("unknown_member".into(), json!(true));
            assert!(
                serde_json::from_value::<$ty>(value).is_err(),
                stringify!($ty)
            );
        }};
    }
    let v = vector("realtime.json");
    closed!(TokenRequest, v["token_request"].clone());
    closed!(CallStartedRequest, v["call_started"].clone());
    closed!(CallEndedRequest, v["call_ended"].clone());
    closed!(WatchRequest, v["watch"].clone());
    closed!(StreamRequest, v["stream_request"].clone());
    closed!(AudioFrame, v["audio_frame"].clone());
    closed!(EndFrame, v["end_frame"].clone());
    for case in vector("kinds.json").as_array().unwrap().iter().take(4) {
        match case["kind"].as_str().unwrap() {
            "transcribe" => closed!(TranscribeInput, case["input"].clone()),
            "synthesize" => closed!(SynthesizeInput, case["input"].clone()),
            "image" => closed!(ImageInput, case["input"].clone()),
            "video" => closed!(VideoInput, case["input"].clone()),
            _ => unreachable!(),
        }
    }
    let chunks = vector("chunks.json");
    closed!(UploadBeginRequest, chunks["lifecycle"]["begin"].clone());
    closed!(UploadAppendRequest, chunks["lifecycle"]["append"].clone());
    closed!(ReadRequest, chunks["lifecycle"]["read"].clone());
    closed!(UploadStatusRequest, json!({"upload_id":"u"}));
    closed!(UploadCommitRequest, json!({"upload_id":"u"}));
    closed!(UsageReadRequest, json!({"limit":1}));
    closed!(UsageAckRequest, json!({"through_seq":0}));
    closed!(ContextRequest, json!({"agent_id":"a","purpose":"bias"}));
    closed!(
        SubmitRequest,
        json!({"client_job_id":"c","kind":"image","input":{"prompt":"a tree","size":"512x512"},"model":"m"})
    );
    closed!(StatusRequest, json!({"job_id":"j"}));
    closed!(ResultRequest, json!({"job_id":"j"}));
    closed!(CancelRequest, json!({"job_id":"j"}));
    for field in ["client_job_id", "stream_id", "call_id"] {
        check_caller_id(&"😀".repeat(128), field).unwrap();
        assert!(check_caller_id(&"😀".repeat(129), field).is_err());
        assert!(check_caller_id("", field).is_err());
    }
}

#[test]
fn media_contract_mime_and_language_matching() {
    assert!(mime_matches(
        "audio/L16;rate=16000;channels=1",
        "Audio/l16;channels=1;rate=16000;extra=yes"
    ));
    assert!(!mime_matches(
        "audio/L16;rate=16000;channels=1",
        "audio/L16;rate=8000;channels=1"
    ));
    assert!(!mime_matches("audio/L16;rate=16000", "audio/L16"));
    assert!(language_matches("en", "EN-gb"));
    assert!(!language_matches("en-GB", "en"));
    assert!(!language_matches("en", "english"));
}

#[test]
fn media_contract_vector_inventory_and_sha256() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("test-vectors/media-v1");
    let mut actual: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != "SHA256SUMS")
        .collect();
    actual.sort();
    assert_eq!(
        actual,
        vectors::FILES
            .iter()
            .map(|(name, _)| name.to_string())
            .collect::<Vec<_>>()
    );
    let expected: String = vectors::FILES
        .iter()
        .map(|(name, content)| format!("{:x}  {name}\n", Sha256::digest(content.as_bytes())))
        .collect();
    assert_eq!(expected, vectors::SHA256SUMS);
    for (name, content) in vectors::FILES {
        assert_eq!(
            std::fs::read(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("test-vectors/media-v1")
                    .join(name)
            )
            .unwrap(),
            content.as_bytes()
        );
    }
}

#[test]
fn media_contract_every_contract_rule_has_a_unique_media_marker() {
    let contract = include_str!("../CONTRACT.md");
    let mut ids = std::collections::BTreeSet::new();
    for line in contract.lines().filter(|line| line.starts_with("- [")) {
        assert!(
            line.starts_with("- [pinned] `media.") || line.starts_with("- [informative] `media."),
            "{line}"
        );
        let id = line.split('`').nth(1).unwrap();
        assert!(ids.insert(id), "duplicate {id}");
        assert!(id.split('.').skip(1).all(|segment| !segment.is_empty()
            && segment
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')));
    }
    assert!(ids.len() >= 40);
}
