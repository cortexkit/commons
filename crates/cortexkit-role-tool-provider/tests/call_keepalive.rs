use cortexkit_role_tool_provider::{
    catalog::{reply_deadline_ms, reply_deadline_ms_with_keepalive, Reply, REPLY_MAX_MS},
    is_keepalive, keepalive_body, vectors, CALL_KEEPALIVE_CAPABILITY, KEEPALIVE_INTERVAL_MS,
    KEEPALIVE_SILENCE_MS,
};
use serde_json::Value;

fn samples() -> Value {
    serde_json::from_str(vectors::CALL_KEEPALIVE_JSON).unwrap()
}

#[test]
fn call_keepalive_constants_and_body_match_samples() {
    let samples = samples();
    assert_eq!(
        CALL_KEEPALIVE_CAPABILITY,
        samples["capability"].as_str().unwrap()
    );
    assert_eq!(
        KEEPALIVE_INTERVAL_MS,
        samples["interval_ms"].as_u64().unwrap()
    );
    assert_eq!(
        KEEPALIVE_SILENCE_MS,
        samples["silence_ms"].as_u64().unwrap()
    );
    assert_eq!(REPLY_MAX_MS, samples["reply_max_ms"].as_u64().unwrap());
    assert_eq!(
        keepalive_body(),
        samples["body"].as_str().unwrap().as_bytes()
    );
}

#[test]
fn call_keepalive_recognition_matches_samples() {
    for case in samples()["recognition"].as_array().unwrap() {
        assert_eq!(
            is_keepalive(case["body"].as_str().unwrap().as_bytes()),
            case["expected"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
    assert!(!is_keepalive(b"\xff"), "invalid UTF-8 is not JSON");
}

#[test]
fn call_keepalive_deadlines_match_samples() {
    for case in samples()["deadlines"].as_array().unwrap() {
        let reply: Option<Reply> = serde_json::from_value(case["reply"].clone()).unwrap();
        let fallback = case["runner_fallback_ms"].as_u64().unwrap();
        for (keepalive, expected) in [(false, "without_keepalive_ms"), (true, "with_keepalive_ms")]
        {
            assert_eq!(
                reply_deadline_ms_with_keepalive(
                    reply.as_ref(),
                    &case["arguments"],
                    fallback,
                    keepalive
                ),
                case[expected].as_u64().unwrap(),
                "{}: keepalive={keepalive}",
                case["name"]
            );
        }
        assert_eq!(
            reply_deadline_ms(reply.as_ref(), &case["arguments"], fallback),
            case["without_keepalive_ms"].as_u64().unwrap(),
            "{}: original helper remains unchanged",
            case["name"]
        );
    }
}
