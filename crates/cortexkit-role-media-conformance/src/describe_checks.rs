use cortexkit_role_media::describe::check_describe_value;
use serde_json::json;

pub(crate) fn checker_codes() -> Result<(), String> {
    let base = json!({"groups":["jobs","usage_feed"],"jobs_survive_restart":false,"engines":[{
        "kind":"transcribe","provider":"fixture","model":"model","local":false,
        "formats":["audio/wav"],"latency_class":"batch"
    }]});
    let mut providers = base.clone();
    let mut gateway = providers["engines"][0].clone();
    gateway["provider"] = json!("gateway");
    providers["engines"].as_array_mut().unwrap().push(gateway);
    check_describe_value(&providers)
        .map_err(|e| format!("describe checker rejected distinct providers: {e:?}"))?;
    for code in [
        "usage_without_usage_feed",
        "stream_without_jobs",
        "voice_front_without_realtime",
        "engine_without_group",
        "group_fact_invalid",
        "duplicate_engine",
        "fact_too_long",
        "bias_limit_invalid",
        "formats_empty",
        "local_realtime_engine",
        "describe_too_large",
    ] {
        let mut bad = base.clone();
        match code {
            "usage_without_usage_feed" => bad["groups"] = json!(["jobs"]),
            "stream_without_jobs" => bad["groups"] = json!(["stream_transcribe"]),
            "voice_front_without_realtime" => bad["groups"] = json!(["voice-front/v1"]),
            "engine_without_group" => bad["groups"] = json!(["usage_feed"]),
            "group_fact_invalid" => bad["jobs_survive_restart"] = json!("false"),
            "duplicate_engine" => {
                let e = bad["engines"][0].clone();
                bad["engines"].as_array_mut().unwrap().push(e);
            }
            "fact_too_long" => bad["engines"][0]["model"] = json!("é".repeat(257)),
            "bias_limit_invalid" => {
                bad["engines"][0]["bias_limit"] = json!({"unit":"phrase","max":0})
            }
            "formats_empty" => bad["engines"][0]["formats"] = json!([]),
            "local_realtime_engine" => {
                bad["groups"] = json!(["realtime"]);
                bad["token_max_ttl_ms"] = json!(1000);
                bad["call_silence_timeout_s"] = json!(1);
                bad["engines"][0]["kind"] = json!("realtime");
                bad["engines"][0]["local"] = json!(true);
            }
            "describe_too_large" => bad["padding"] = json!("x".repeat(3_145_729)),
            _ => unreachable!(),
        }
        match check_describe_value(&bad) {
            Err(e) if e.code == code => (),
            other => return Err(format!("describe checker expected {code}, got {other:?}")),
        }
    }
    Ok(())
}
