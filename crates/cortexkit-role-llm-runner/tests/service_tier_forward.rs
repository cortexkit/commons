use cortexkit_role_llm_runner::usage::RunUsage;
use serde_json::json;

#[test]
fn record_reader_preserves_unknown_applied_tier() {
    let raw = json!({"steps":0,"unreported_steps":0,"by_model":[],
        "service_tier_applied":"future-tier", "service_tier_served":"future-provider-tier"});
    let usage: RunUsage = serde_json::from_value(raw.clone()).unwrap();
    assert!(usage.consistent());
    assert_eq!(serde_json::to_value(usage).unwrap(), raw);
}
