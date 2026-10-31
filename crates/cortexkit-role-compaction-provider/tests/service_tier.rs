use std::collections::BTreeMap;

use cortexkit_role_compaction_provider::{
    notice::{headers, AdvanceNotice, Event, NoticeProblem},
    ride::{RideCause, RideProblem, RideRequest},
};
use cortexkit_role_llm_runner::service_tier::ServiceTier;
use serde_json::{json, Value};

fn samples() -> Value {
    serde_json::from_str(include_str!("../test-vectors/service-tier-v1/cases.json")).unwrap()
}

#[test]
fn service_tier_ride_carries_the_level_like_an_account_label() {
    let raw = samples()["ride"].clone();
    let ride: RideRequest = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(ride.cause, RideCause::ServiceTierChange);
    assert_eq!(ride.service_tier(), Some(ServiceTier::Fast));
    assert_eq!(ride.check(), Ok(()));
    assert_eq!(serde_json::to_value(&ride).unwrap(), raw);
    for level in [
        ServiceTier::Standard,
        ServiceTier::Fast,
        ServiceTier::Ultrafast,
    ] {
        let ride = RideRequest::new("a".repeat(32), RideCause::ServiceTierChange, 1000)
            .with_service_tier(level);
        assert_eq!(ride.check(), Ok(()));
        assert_eq!(
            serde_json::to_value(&ride).unwrap()["service_tier"],
            level.as_str()
        );
        let token = format!("service_tier_change:{}", level.as_str());
        assert_eq!(
            Event::parse(&token).unwrap(),
            Event::ServiceTierChange(level.as_str().into())
        );
        assert_eq!(Event::parse(&token).unwrap().token(), token);
    }
    for value in [None, Some("rapid".into()), Some("".into())] {
        let mut ride = ride.clone();
        ride.service_tier = value;
        assert_eq!(ride.check(), Err(RideProblem::ServiceTier));
        ride.cause = RideCause::ModelChange;
        assert_eq!(ride.check(), Ok(()), "irrelevant tier members are ignored");
        assert_eq!(ride.service_tier(), None);
    }
    assert!(Event::parse("service_tier_change").is_err());
    assert_eq!(
        Event::parse("service_tier_change:rapid").unwrap(),
        Event::ServiceTierChange("rapid".into())
    );
    assert_eq!(
        RideCause::parse("future_cause"),
        RideCause::Unknown("future_cause".into())
    );
}

#[test]
fn service_tier_notices_digest_the_new_condition_and_preserve_legacy_bytes() {
    let data = samples();
    let without: BTreeMap<String, String> =
        serde_json::from_value(data["notice_without"].clone()).unwrap();
    let with: BTreeMap<String, String> =
        serde_json::from_value(data["notice_with"].clone()).unwrap();
    let legacy = AdvanceNotice::from_headers(&without).unwrap();
    assert_eq!(legacy.to_headers(), without);
    assert_eq!(
        legacy
            .clone()
            .with_service_tier(ServiceTier::Standard)
            .to_headers(),
        without
    );
    assert_eq!(
        legacy.digest(),
        "8ce7d14204dd4df5bbcdd9fda1bb8ef8620c2493c4d9c3d08d59c4cf581651f0"
    );
    let fast = AdvanceNotice::from_headers(&with).unwrap();
    assert_eq!(fast.to_headers(), with);
    assert_eq!(
        legacy.with_service_tier(ServiceTier::Fast).to_headers(),
        with
    );
    assert_eq!(
        fast.digest(),
        "822052b3dae718fcc0a69e699d1db9a388d33893fd90033fba5de7ff68e2b8cc"
    );
    for value in ["standard", "rapid", ""] {
        let mut headers = with.clone();
        headers.insert(headers::SERVICE_TIER.into(), value.into());
        let decoded = AdvanceNotice::from_headers(&headers).unwrap();
        assert_eq!(decoded.to_headers(), headers);
        assert_eq!(
            decoded.check(),
            Err(NoticeProblem::Malformed(headers::SERVICE_TIER))
        );
    }
    let legacy_ride = RideRequest::new("a".repeat(32), RideCause::ModelChange, 1000);
    assert_eq!(
        serde_json::to_string(&legacy_ride).unwrap(),
        r#"{"session_ref":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","cause":"model_change","deadline_ms":1000}"#
    );
    assert_eq!(
        serde_json::to_value(legacy_ride).unwrap(),
        json!({"session_ref":"a".repeat(32),"cause":"model_change","deadline_ms":1000})
    );
}

#[test]
fn service_tier_notice_and_ride_readers_preserve_future_levels() {
    let mut headers: BTreeMap<String, String> =
        serde_json::from_value(samples()["notice_with"].clone()).unwrap();
    headers.insert(headers::SERVICE_TIER.into(), "future-tier".into());
    let notice = AdvanceNotice::from_headers(&headers).unwrap();
    assert_eq!(notice.to_headers(), headers);
    assert_eq!(
        notice.digest(),
        "d114221b59562e93ae7604e03efe7882f72f241510058695639e4d2bce209c27"
    );
    assert_eq!(
        notice.check(),
        Err(NoticeProblem::Malformed(headers::SERVICE_TIER))
    );
    headers.remove(headers::SERVICE_TIER);
    headers.insert(
        headers::EVENTS.into(),
        "service_tier_change:future-tier".into(),
    );
    let notice = AdvanceNotice::from_headers(&headers).unwrap();
    assert_eq!(notice.to_headers(), headers);
    assert_eq!(
        notice.check(),
        Err(NoticeProblem::Malformed(headers::EVENTS))
    );
    let mut raw = samples()["ride"].clone();
    raw["service_tier"] = json!("future-tier");
    let ride: RideRequest = serde_json::from_value(raw.clone()).unwrap();
    assert_eq!(serde_json::to_value(&ride).unwrap(), raw);
    assert_eq!(ride.check(), Err(RideProblem::ServiceTier));
}
