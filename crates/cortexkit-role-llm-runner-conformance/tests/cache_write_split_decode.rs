use cortexkit_role_llm_runner::run::{RunResult, RunStatus};
use serde_json::{json, Value};

struct Decoded {
    body: Value,
    split: Option<(u64, u64)>,
    consistent: bool,
}

fn decode(surface: &str, scope: &str, split: Value) -> Result<Decoded, serde_json::Error> {
    let mut usage = json!({
        "steps": 1,
        "unreported_steps": 0,
        "cache_write_tokens": 10,
        "by_model": [{
            "provider": "provider", "model": "model", "steps": 1,
            "unreported_steps": 0, "cache_write_tokens": 10
        }]
    });
    if let Some(total) = split["cache_write_tokens"].as_u64() {
        usage["cache_write_tokens"] = total.into();
        usage["by_model"][0]["cache_write_tokens"] = total.into();
    }
    let target = if scope == "run" {
        &mut usage
    } else {
        &mut usage["by_model"][0]
    };
    target
        .as_object_mut()
        .unwrap()
        .extend(split.as_object().unwrap().clone());
    let body = json!({"run_id": "run", "state": "completed", "usage": usage});
    let (usage, body) = if surface == "status" {
        let status: RunStatus = serde_json::from_value(body)?;
        (status.usage.clone().unwrap(), serde_json::to_value(status)?)
    } else {
        let result: RunResult = serde_json::from_value(body)?;
        (result.usage.clone().unwrap(), serde_json::to_value(result)?)
    };
    let split = if scope == "run" {
        usage.cache_write_split()
    } else {
        usage.by_model[0].cache_write_split()
    };
    Ok(Decoded {
        body,
        split,
        consistent: usage.consistent(),
    })
}

fn target<'a>(body: &'a Value, scope: &str) -> &'a Value {
    if scope == "run" {
        &body["usage"]
    } else {
        &body["usage"]["by_model"][0]
    }
}

macro_rules! decoder_cases {
    ($module:ident, $surface:literal, $scope:literal) => {
        mod $module {
            use super::*;

            #[test]
            fn both_absent() {
                let decoded = decode($surface, $scope, json!({})).unwrap();
                assert_eq!(decoded.split, None);
                assert!(decoded.consistent);
                let usage = target(&decoded.body, $scope);
                assert_eq!(usage["cache_write_tokens"], 10);
                assert!(usage.get("cache_write_5m_tokens").is_none());
                assert!(usage.get("cache_write_1h_tokens").is_none());
            }

            #[test]
            fn both_present_and_summing() {
                for (total, five, hour) in [(10, 3, 7), (10, 0, 10), (0, 0, 0), (u64::MAX, u64::MAX, 0)] {
                    let decoded = decode($surface, $scope, json!({
                        "cache_write_tokens": total,
                        "cache_write_5m_tokens": five,
                        "cache_write_1h_tokens": hour,
                        "future_usage_field": true
                    })).unwrap();
                    assert_eq!(decoded.split, Some((five, hour)));
                    assert!(decoded.consistent);
                    let usage = target(&decoded.body, $scope);
                    assert_eq!(usage["cache_write_tokens"], total);
                    assert_eq!(usage["cache_write_5m_tokens"], five);
                    assert_eq!(usage["cache_write_1h_tokens"], hour);
                }
            }

            fn ignored(split: Value) {
                let decoded = decode($surface, $scope, split.clone()).expect("malformed optional split must not reject a run reply");
                assert_eq!(decoded.split, None);
                assert!(!decoded.consistent);
                assert_eq!(decoded.body["state"], "completed");
                assert_eq!(decoded.body["usage"]["steps"], 1);
                assert_eq!(decoded.body["usage"]["by_model"][0]["model"], "model");
                assert_eq!(target(&decoded.body, $scope)["cache_write_tokens"], split.get("cache_write_tokens").cloned().unwrap_or(json!(10)));
            }

            #[test]
            fn only_five_minute_ignored() {
                ignored(json!({"cache_write_5m_tokens": 10}));
            }

            #[test]
            fn only_one_hour_ignored() {
                ignored(json!({"cache_write_1h_tokens": 10}));
            }

            #[test]
            fn wrong_sum_ignored() {
                ignored(json!({"cache_write_5m_tokens": 3, "cache_write_1h_tokens": 8}));
            }

            #[test]
            fn missing_total_ignored() {
                ignored(json!({"cache_write_tokens": null, "cache_write_5m_tokens": 3, "cache_write_1h_tokens": 7}));
            }

            #[test]
            fn overflowing_sum_ignored() {
                ignored(json!({"cache_write_tokens": u64::MAX, "cache_write_5m_tokens": u64::MAX, "cache_write_1h_tokens": 1}));
            }
        }
    };
}

decoder_cases!(status_run, "status", "run");
decoder_cases!(status_model, "status", "model");
decoder_cases!(result_run, "result", "run");
decoder_cases!(result_model, "result", "model");
