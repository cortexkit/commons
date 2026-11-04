use cortexkit_role_routing::{
    account::{self, AccountForAnswer, AccountForRequest},
    capabilities,
    service_tier::{self, ServiceTier},
};
use serde_json::json;

#[test]
fn account_for_tier_request_refuses_unknown_by_name() {
    let raw = json!({"provider":"example", "model":"model", "conversation_key":"conversation", "admission_id":"admission", "service_tier":"future"});
    let refusal = account::parse_account_for_request_serving(&raw, &[capabilities::SERVICE_TIER])
        .unwrap_err();
    assert_eq!(refusal.code.as_str(), "invalid_request");
    assert_eq!(refusal.detail.get("field"), Some(&json!("service_tier")));
    assert_eq!(
        refusal.detail.get("reason"),
        Some(&json!("unsupported_tier"))
    );
    assert_eq!(
        account::parse_account_for_request(&raw)
            .unwrap()
            .service_tier,
        None
    );
}

#[test]
fn account_for_tier_legacy_parser_ignores_unknown_members_without_group() {
    let raw = json!({"provider":"example", "model":"model", "conversation_key":"conversation", "admission_id":"admission", "service_tier":"fast", "future_member": true});
    let legacy = account::parse_account_for_request(&raw).unwrap();
    assert_eq!(legacy.service_tier, None);
    let aware =
        account::parse_account_for_request_serving(&raw, &[capabilities::SERVICE_TIER]).unwrap();
    assert_eq!(aware.service_tier, Some(ServiceTier::Fast));
}

#[test]
fn account_for_tier_idempotency_compares_default_and_changed_levels() {
    let request = AccountForRequest::new("example", "model", "conversation", "admission");
    assert!(request.same_body(&request.clone().with_service_tier(ServiceTier::Standard)));
    assert_eq!(request.normalized().service_tier, None);
    assert_eq!(
        request
            .clone()
            .with_service_tier(ServiceTier::Standard)
            .normalized()
            .service_tier,
        None
    );
    assert!(!request.same_body(&request.clone().with_service_tier(ServiceTier::Fast)));
    assert_eq!(
        service_tier::fallback(ServiceTier::Ultrafast).collect::<Vec<_>>(),
        vec![
            ServiceTier::Ultrafast,
            ServiceTier::Fast,
            ServiceTier::Standard
        ]
    );
    assert!(service_tier::check_chosen(
        Some(ServiceTier::Fast),
        Some("standard")
    ));
    assert!(!service_tier::check_chosen(Some(ServiceTier::Fast), None));
    assert!(!service_tier::check_chosen(
        Some(ServiceTier::Fast),
        Some("ultrafast")
    ));
    assert!(!service_tier::check_chosen(
        Some(ServiceTier::Fast),
        Some("future")
    ));
}

#[test]
fn account_for_tier_legacy_wire_bytes_round_trip() {
    let request = r#"{"provider":"example","model":"model","conversation_key":"conversation","admission_id":"admission"}"#;
    let parsed =
        account::parse_account_for_request(&serde_json::from_str(request).unwrap()).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), request);
    let answer = r#"{"admission":{"id":"admission","state":"admitted"},"account":{"provider":"example","account_id":"account","credential_id":"credential","method":"oauth"},"pin":{"id":"pin","generation":1,"affinity_expires_at_ms":3,"admission_valid_until_ms":2,"lease_valid_until_ms":1,"mode":"admit"},"reason":{"code":"affinity","decision_id":"decision","policy_version":"1","quota_age_band":"fresh"}}"#;
    let parsed: AccountForAnswer = serde_json::from_str(answer).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), answer);
    let new = parsed.with_service_tier_chosen(ServiceTier::Fast);
    assert_eq!(
        serde_json::to_value(new).unwrap()["service_tier_chosen"],
        "fast"
    );
}
