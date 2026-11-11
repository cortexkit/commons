use cortexkit_role_compaction::{
    notice::{Conditions, NoticeRequest},
    provides_statement_seq,
    ride::RideRequest,
    rules::{EditCost, ProviderRules, SignedParts},
    setup::{ModelRef, SetupRequest},
    wire, COMPACTION_PROVIDES, COMPACTION_STATEMENT_SEQ_PROVIDES,
};
use serde_json::{json, Value};

fn conditions() -> Conditions {
    Conditions::new(
        ModelRef::new("example-provider", "example-model"),
        200_000,
        8_000,
        ProviderRules::new(SignedParts::Bound, EditCost::Suffix),
    )
}

fn setup() -> SetupRequest {
    let conditions = conditions();
    SetupRequest::new(
        "example",
        conditions.model,
        conditions.context_window,
        conditions.output_reserve,
        conditions.rules,
        vec![],
        1,
    )
}

#[test]
fn statement_seq_preserves_zero_equal_lower_and_safe_integer_limit() {
    // Equal stamps describe the same state; a Setup may reset the held stamp
    // after a restore, so wire decoding must also preserve a lower value.
    for stamp in [0, 42, 42, 7, 9_007_199_254_740_991_u64] {
        let mut conditions_json = serde_json::to_value(conditions()).unwrap();
        let mut setup_json = serde_json::to_value(setup()).unwrap();
        conditions_json["statement_seq"] = json!(stamp);
        setup_json["statement_seq"] = json!(stamp);
        let decoded: Conditions = serde_json::from_value(conditions_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), conditions_json);
        let decoded: SetupRequest = serde_json::from_value(setup_json.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), setup_json);
    }
}

#[test]
fn statement_seq_projects_only_for_declaring_providers() {
    assert_eq!(
        COMPACTION_STATEMENT_SEQ_PROVIDES,
        "compaction-statement-seq/v1"
    );
    for declared in [false, true] {
        let mut provides = vec![COMPACTION_PROVIDES.to_owned()];
        if declared {
            provides.push(COMPACTION_STATEMENT_SEQ_PROVIDES.to_owned());
        }
        assert_eq!(provides_statement_seq(&provides), declared);
        let conditions = conditions().with_statement_seq(42, provides_statement_seq(&provides));
        let setup = setup().with_statement_seq(42, provides_statement_seq(&provides));
        assert_eq!(conditions.statement_seq, declared.then_some(42));
        assert_eq!(setup.statement_seq, declared.then_some(42));
        for value in [
            serde_json::to_value(&conditions).unwrap(),
            serde_json::to_value(&setup).unwrap(),
        ] {
            assert_eq!(value.get("statement_seq"), declared.then_some(&json!(42)));
        }
        assert!(conditions
            .with_statement_seq(43, false)
            .statement_seq
            .is_none());
        assert!(setup.with_statement_seq(43, false).statement_seq.is_none());
    }
    for undeclared in [
        "compaction-statement-seq/v2",
        "compaction-statement-seq/v1-extra",
        "compaction-signed-floor/v1",
    ] {
        assert!(!provides_statement_seq(&[undeclared.into()]));
    }
}

#[test]
fn statement_seq_is_optional_and_decoders_still_ignore_unknown_members() {
    let mut conditions_json = serde_json::to_value(conditions()).unwrap();
    let mut setup_json = serde_json::to_value(setup()).unwrap();
    for value in [&mut conditions_json, &mut setup_json] {
        assert!(value.get("statement_seq").is_none());
        value["future_member"] = json!({"unknown": true});
    }
    for explicit_null in [false, true] {
        if explicit_null {
            conditions_json["statement_seq"] = Value::Null;
            setup_json["statement_seq"] = Value::Null;
        }
        let decoded: Conditions = serde_json::from_value(conditions_json.clone()).unwrap();
        assert!(decoded.statement_seq.is_none());
        assert_eq!(decoded, conditions());
        let decoded: SetupRequest = serde_json::from_value(setup_json.clone()).unwrap();
        assert!(decoded.statement_seq.is_none());
        assert_eq!(decoded, setup());
    }
}

#[test]
fn statement_seq_rejects_invalid_wire_integers_with_field_path() {
    for invalid in [
        json!(-1),
        json!(1.5),
        json!("42"),
        json!(true),
        json!({}),
        json!(9_007_199_254_740_992_u64),
        json!(u64::MAX),
    ] {
        let mut conditions_json = serde_json::to_value(conditions()).unwrap();
        let mut setup_json = serde_json::to_value(setup()).unwrap();
        conditions_json["statement_seq"] = invalid.clone();
        setup_json["statement_seq"] = invalid.clone();
        for refusal in [
            wire::params::<Conditions>(conditions_json).unwrap_err(),
            wire::params::<SetupRequest>(setup_json).unwrap_err(),
        ] {
            assert_eq!(refusal.code, wire::codes::INVALID_PARAMS);
            assert_eq!(refusal.detail.unwrap()["field"], "statement_seq");
        }
    }
}

#[test]
fn statement_seq_vectors_keep_equal_state_and_lower_restore_stamp() {
    let setup: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/setup-statement-seq-request.json"
    ))
    .unwrap();
    let notice: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/notice-statement-seq-request.json"
    ))
    .unwrap();
    let ride: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/ride-statement-seq-request.json"
    ))
    .unwrap();
    let restored: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/setup-statement-seq-restored-request.json"
    ))
    .unwrap();
    let setup: SetupRequest = wire::params(setup["params"].clone()).unwrap();
    let notice: NoticeRequest = wire::params(notice["params"].clone()).unwrap();
    let ride: RideRequest = wire::params(ride["params"].clone()).unwrap();
    let restored: SetupRequest = wire::params(restored["params"].clone()).unwrap();
    assert_eq!(setup.statement_seq, Some(42));
    assert_eq!(notice.conditions.statement_seq, Some(42));
    assert_eq!(ride.conditions.statement_seq, Some(42));
    assert_eq!(setup.signed_floor, notice.conditions.signed_floor);
    assert_eq!(setup.signed_floor, ride.conditions.signed_floor);
    assert_eq!(restored.statement_seq, Some(7));
    assert!(restored.signed_floor.is_none());
}
