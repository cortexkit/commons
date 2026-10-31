use cortexkit_role_llm_runner::service_tier::ServiceTier;
use cortexkit_usage_steps::StepRow;
use serde_json::Value;

#[test]
fn service_tier_row_producer_spellings_do_not_drift_from_runner() {
    let expected: Vec<_> = ServiceTier::ALL
        .iter()
        .filter(|tier| !tier.is_standard())
        .map(|tier| tier.as_str())
        .collect();
    assert_eq!(StepRow::SERVICE_TIER_APPLIED_LEVELS, expected);
    let raw: Value = serde_json::from_str(include_str!(
        "../test-vectors/usage-steps-v1/read/row-every-member.json"
    ))
    .unwrap();
    for tier in [
        ServiceTier::Standard,
        ServiceTier::Fast,
        ServiceTier::Ultrafast,
    ] {
        let mut raw = raw.clone();
        raw["service_tier_applied"] = Value::String(tier.as_str().into());
        assert_eq!(StepRow::check_closed(&raw).is_ok(), !tier.is_standard());
    }
}

#[test]
fn service_tier_row_reader_keeps_unknown_levels_but_producers_refuse_them() {
    let mut raw: Value = serde_json::from_str(include_str!(
        "../test-vectors/usage-steps-v1/read/row-every-member.json"
    ))
    .unwrap();
    raw["service_tier_applied"] = Value::String("future-tier".into());
    let row: StepRow = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(row).unwrap(), raw);
    assert!(StepRow::check_closed(&raw).is_err());
}

#[test]
fn service_tier_rows_preserve_legacy_bytes_and_truthful_usage_samples() {
    let legacy = include_str!("../test-vectors/usage-steps-v1/read/row-every-member.json").trim();
    let row: StepRow = serde_json::from_str(legacy).unwrap();
    assert_eq!(serde_json::to_string(&row).unwrap(), legacy);
    assert_eq!(
        serde_json::to_string(
            &row.clone()
                .with_service_tier_applied(ServiceTier::Standard.as_str())
        )
        .unwrap(),
        legacy
    );
    let samples: Value =
        serde_json::from_str(include_str!("../test-vectors/service-tier-v1/cases.json")).unwrap();
    for usage in samples["usage"].as_array().unwrap() {
        let mut raw = serde_json::to_value(&row).unwrap();
        raw.as_object_mut()
            .unwrap()
            .extend(usage.as_object().unwrap().clone());
        let decoded: StepRow = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(StepRow::check_closed(&raw), Ok(()));
        assert_eq!(serde_json::to_value(decoded).unwrap(), raw);
    }
}
