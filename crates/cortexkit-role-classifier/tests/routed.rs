use cortexkit_role_classifier::{
    describe::{Limits, ModelEntry, RoleDescribe},
    errors::{self, RoutingRefusal},
    identity::BatchIdentity,
    reply::{check_reply, ClassifyReply},
    request::{parse_request, parse_request_for, ClassifyRequest},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, Serialize)]
struct RoutedVector {
    request: ClassifyRequest,
    reply: ClassifyReply,
    refusals: Vec<RoutingRefusal>,
}

#[test]
fn routed_models_vector() {
    let text = include_str!("../test-vectors/classifier-v1/routed-models.json");
    let vector: RoutedVector = serde_json::from_str(text).unwrap();
    assert_eq!(
        serde_json::to_value(&vector).unwrap(),
        serde_json::from_str::<Value>(text).unwrap()
    );
    assert_eq!(vector.request.model, None);
    check_reply(&vector.reply, &vector.request).unwrap();
    let mut unnamed = vector.reply.clone();
    unnamed.model.clear();
    assert!(check_reply(&unnamed, &vector.request)
        .unwrap_err()
        .contains("model"));
    let mut missing = vector.reply.clone();
    missing.decision_id = None;
    assert!(check_reply(&missing, &vector.request)
        .unwrap_err()
        .contains("decision_id"));
    let mut pinned = vector.request.clone();
    pinned.model = Some("other/model".into());
    assert!(check_reply(&vector.reply, &pinned)
        .unwrap_err()
        .contains("model"));
    let unpinned_identity = BatchIdentity::of(&vector.request).unwrap();
    assert_eq!(
        unpinned_identity
            .first_difference(&BatchIdentity::of(&pinned).unwrap())
            .as_deref(),
        Some("model")
    );
    let built = [
        RoutingRefusal::refused(
            "no_adequate_model",
            Some("no model meets the requirements".into()),
            None,
        ),
        RoutingRefusal::refused(
            "pin_unavailable",
            Some("no account can serve the pin".into()),
            Some("typesafe/jev-1.13.0".into()),
        ),
        RoutingRefusal::unavailable("routing timed out"),
    ];
    assert_eq!(vector.refusals, built);
    for refusal in vector.refusals {
        assert!(errors::ADMISSION_CODES.contains(&refusal.code.as_str()));
        assert_eq!(
            errors::admission_class(&refusal.code),
            Some(refusal.detail.class)
        );
    }
    assert_eq!(
        parse_request(&serde_json::to_string(&vector.request).unwrap()).unwrap(),
        vector.request
    );
    let mut malformed = serde_json::to_value(&pinned).unwrap();
    malformed["model"] = json!(12);
    assert_eq!(
        parse_request(&malformed.to_string()).unwrap_err().field(),
        Some("model")
    );
}

#[test]
fn routed_describe_is_opt_in_and_backward_decodable() {
    let legacy = RoleDescribe::new(
        Limits::new(10, 10000),
        vec![ModelEntry::new(
            "example/model",
            8,
            10,
            8192,
            "example.invalid",
        )],
    );
    let legacy_wire = serde_json::to_value(&legacy).unwrap();
    let text = json!({"batch_id":"model-choice", "questions":{"q":{"type":"noul","instructions":"Test"}}, "items":[{"state":"item"}]}).to_string();
    assert_eq!(
        parse_request_for(&text, &legacy).unwrap_err().field(),
        Some("model")
    );
    assert!(legacy_wire.get("routed_models").is_none());
    assert!(!serde_json::from_value::<RoleDescribe>(legacy_wire)
        .unwrap()
        .declares_routed_models());
    let routed = legacy.with_routed_models();
    assert_eq!(parse_request_for(&text, &routed).unwrap().model, None);
    let wire = serde_json::to_value(&routed).unwrap();
    assert_eq!(wire["routed_models"], json!({}));
    assert!(serde_json::from_value::<RoleDescribe>(wire.clone())
        .unwrap()
        .declares_routed_models());
    // The old describe decoder had no deny_unknown_fields attribute.
    #[derive(Deserialize)]
    struct LegacyDescribe {
        role: String,
        limits: Limits,
        models: Vec<ModelEntry>,
    }
    let old: LegacyDescribe = serde_json::from_value(wire).unwrap();
    assert_eq!(old.role, routed.role);
    assert_eq!(old.limits, routed.limits);
    assert_eq!(old.models, routed.models);
}
