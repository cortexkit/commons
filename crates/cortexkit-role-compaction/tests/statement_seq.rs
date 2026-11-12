use cortexkit_role_compaction::{
    notice::{Conditions, NoticeRequest},
    provides_statement_seq,
    ride::RideRequest,
    rules::{EditCost, ProviderRules, SignedParts},
    setup::{ModelRef, SetupRequest},
    wire, COMPACTION_PROVIDES, COMPACTION_STATEMENT_SEQ_PROVIDES,
};
use serde_json::{json, Value};

const EPOCH: &str = "89abcdef0123456789abcdef01234567";
const RESTORED_EPOCH: &str = "fedcba9876543210fedcba9876543210";

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
fn statement_epoch_rejects_invalid_wire_values_with_field_path() {
    for invalid in [
        json!(""),
        json!("a".repeat(65)),
        json!("has space"),
        json!("line\nbreak"),
        json!("\u{7f}"),
        json!("é"),
        json!(42),
    ] {
        let mut conditions_json = serde_json::to_value(conditions()).unwrap();
        let mut setup_json = serde_json::to_value(setup()).unwrap();
        conditions_json["statement_epoch"] = invalid.clone();
        setup_json["statement_epoch"] = invalid;
        for refusal in [
            wire::params::<Conditions>(conditions_json).unwrap_err(),
            wire::params::<SetupRequest>(setup_json).unwrap_err(),
        ] {
            assert_eq!(refusal.code, wire::codes::INVALID_PARAMS);
            assert_eq!(refusal.detail.unwrap()["field"], "statement_epoch");
        }
    }
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
        conditions_json["statement_epoch"] = json!(EPOCH);
        setup_json["statement_epoch"] = json!(EPOCH);
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
        let conditions = conditions()
            .with_statement_order(EPOCH, 42, provides_statement_seq(&provides))
            .unwrap();
        let setup = setup()
            .with_statement_order(EPOCH, 42, provides_statement_seq(&provides))
            .unwrap();
        assert_eq!(conditions.statement_seq, declared.then_some(42));
        assert_eq!(setup.statement_seq, declared.then_some(42));
        assert_eq!(
            conditions.statement_epoch.as_deref(),
            declared.then_some(EPOCH)
        );
        assert_eq!(setup.statement_epoch.as_deref(), declared.then_some(EPOCH));
        for value in [
            serde_json::to_value(&conditions).unwrap(),
            serde_json::to_value(&setup).unwrap(),
        ] {
            assert_eq!(value.get("statement_seq"), declared.then_some(&json!(42)));
            assert_eq!(
                value.get("statement_epoch"),
                declared.then_some(&json!(EPOCH))
            );
        }
        let conditions = conditions.with_statement_order(EPOCH, 43, false).unwrap();
        let setup = setup.with_statement_order(EPOCH, 43, false).unwrap();
        assert!(conditions.statement_seq.is_none());
        assert!(conditions.statement_epoch.is_none());
        assert!(setup.statement_seq.is_none());
        assert!(setup.statement_epoch.is_none());
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
        assert!(value.get("statement_epoch").is_none());
        value["future_member"] = json!({"unknown": true});
    }
    for explicit_null in [false, true] {
        if explicit_null {
            conditions_json["statement_seq"] = Value::Null;
            setup_json["statement_seq"] = Value::Null;
            conditions_json["statement_epoch"] = Value::Null;
            setup_json["statement_epoch"] = Value::Null;
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
fn statement_epoch_validates_builders_and_encoding_and_round_trips_boundaries() {
    for invalid in [
        "".to_owned(),
        "a".repeat(65),
        "has space".into(),
        "\u{1f}".into(),
        "\u{7f}".into(),
        "é".into(),
    ] {
        for declared in [false, true] {
            assert!(conditions()
                .with_statement_order(&invalid, 42, declared)
                .is_err());
            assert!(setup()
                .with_statement_order(&invalid, 42, declared)
                .is_err());
        }
        let mut conditions = conditions();
        let mut setup = setup();
        conditions.statement_epoch = Some(invalid.clone());
        setup.statement_epoch = Some(invalid);
        assert!(serde_json::to_value(conditions).is_err());
        assert!(serde_json::to_value(setup).is_err());
    }
    for epoch in ["!".to_owned(), "~".repeat(64), EPOCH.into()] {
        let conditions = conditions().with_statement_order(&epoch, 42, true).unwrap();
        let setup = setup().with_statement_order(&epoch, 42, true).unwrap();
        let decoded: Conditions =
            serde_json::from_value(serde_json::to_value(&conditions).unwrap()).unwrap();
        assert_eq!(decoded, conditions);
        let decoded: SetupRequest =
            serde_json::from_value(serde_json::to_value(&setup).unwrap()).unwrap();
        assert_eq!(decoded, setup);
    }
}

#[test]
fn statement_seq_without_epoch_decodes_for_drop_but_cannot_be_encoded() {
    // No builder sets a stamp without its epoch; only decoding or a direct
    // field write can hold one.
    let mut legacy_conditions = conditions();
    legacy_conditions.statement_seq = Some(42);
    let mut legacy_setup = setup();
    legacy_setup.statement_seq = Some(42);
    for error in [
        serde_json::to_value(legacy_conditions).unwrap_err(),
        serde_json::to_value(legacy_setup).unwrap_err(),
    ] {
        assert!(error
            .to_string()
            .contains("statement_seq requires statement_epoch"));
    }
    // An undeclared projection clears both members.
    let legacy_conditions = conditions()
        .with_statement_order(EPOCH, 42, true)
        .unwrap()
        .with_statement_order(EPOCH, 43, false)
        .unwrap();
    let legacy_setup = setup()
        .with_statement_order(EPOCH, 42, true)
        .unwrap()
        .with_statement_order(EPOCH, 43, false)
        .unwrap();
    for value in [
        serde_json::to_value(legacy_conditions).unwrap(),
        serde_json::to_value(legacy_setup).unwrap(),
    ] {
        assert!(value.get("statement_epoch").is_none());
        assert!(value.get("statement_seq").is_none());
    }
    let mut conditions_json = serde_json::to_value(conditions()).unwrap();
    let mut setup_json = serde_json::to_value(setup()).unwrap();
    conditions_json["statement_seq"] = json!(42);
    setup_json["statement_seq"] = json!(42);
    let conditions: Conditions = wire::params(conditions_json).unwrap();
    let setup: SetupRequest = wire::params(setup_json).unwrap();
    assert_eq!(conditions.statement_seq, Some(42));
    assert_eq!(setup.statement_seq, Some(42));
    assert!(conditions.statement_epoch.is_none());
    assert!(setup.statement_epoch.is_none());
    assert!(serde_json::to_value(conditions).is_err());
    assert!(serde_json::to_value(setup).is_err());
}

#[test]
fn statement_epoch_mixed_presence_decodes_without_losing_the_epoch() {
    for epoch in [None, Some(EPOCH)] {
        for stamp in [None, Some(42)] {
            let mut conditions_json = serde_json::to_value(conditions()).unwrap();
            let mut setup_json = serde_json::to_value(setup()).unwrap();
            for value in [&mut conditions_json, &mut setup_json] {
                if let Some(epoch) = epoch {
                    value["statement_epoch"] = json!(epoch);
                }
                if let Some(stamp) = stamp {
                    value["statement_seq"] = json!(stamp);
                }
            }
            let conditions: Conditions = wire::params(conditions_json).unwrap();
            let setup: SetupRequest = wire::params(setup_json).unwrap();
            assert_eq!(conditions.statement_epoch.as_deref(), epoch);
            assert_eq!(setup.statement_epoch.as_deref(), epoch);
            assert_eq!(conditions.statement_seq, stamp);
            assert_eq!(setup.statement_seq, stamp);
        }
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
    let next: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/notice-statement-seq-restored-request.json"
    ))
    .unwrap();
    let setup: SetupRequest = wire::params(setup["params"].clone()).unwrap();
    let notice: NoticeRequest = wire::params(notice["params"].clone()).unwrap();
    let ride: RideRequest = wire::params(ride["params"].clone()).unwrap();
    let restored: SetupRequest = wire::params(restored["params"].clone()).unwrap();
    let next: NoticeRequest = wire::params(next["params"].clone()).unwrap();
    assert_eq!(setup.statement_seq, Some(42));
    assert_eq!(notice.conditions.statement_seq, Some(42));
    assert_eq!(ride.conditions.statement_seq, Some(42));
    assert_eq!(setup.statement_epoch.as_deref(), Some(EPOCH));
    assert_eq!(notice.conditions.statement_epoch, setup.statement_epoch);
    assert_eq!(ride.conditions.statement_epoch, setup.statement_epoch);
    assert_eq!(setup.signed_floor, notice.conditions.signed_floor);
    assert_eq!(setup.signed_floor, ride.conditions.signed_floor);
    assert_eq!(restored.statement_seq, Some(7));
    assert_eq!(restored.statement_epoch.as_deref(), Some(RESTORED_EPOCH));
    assert!(restored.signed_floor.is_none());
    assert_eq!(next.seq, 1);
    assert_eq!(next.conditions.statement_epoch, restored.statement_epoch);
    assert_eq!(next.conditions.statement_seq, Some(8));
}
