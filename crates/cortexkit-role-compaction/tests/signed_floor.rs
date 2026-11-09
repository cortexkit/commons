use cortexkit_role_compaction::{
    notice::Conditions,
    provides_signed_floor,
    rules::{EditCost, ProviderRules, SignedParts},
    setup::{ModelRef, SetupRequest},
    SignedFloor, COMPACTION_PROVIDES, COMPACTION_SIGNED_FLOOR_PROVIDES,
};
use serde_json::json;

fn conditions() -> Conditions {
    Conditions::new(
        ModelRef::new("anthropic", "fixture"),
        200_000,
        8_000,
        ProviderRules::new(SignedParts::Bound, EditCost::Suffix),
    )
}

fn setup() -> SetupRequest {
    let conditions = conditions();
    SetupRequest::new(
        "head",
        conditions.model,
        conditions.context_window,
        conditions.output_reserve,
        conditions.rules,
        vec![],
        1,
    )
}

#[test]
fn signed_floor_projects_only_for_declaring_providers() {
    assert_eq!(
        COMPACTION_SIGNED_FLOOR_PROVIDES,
        "compaction-signed-floor/v1"
    );
    for declared in [false, true] {
        let mut provides = vec![COMPACTION_PROVIDES.to_owned()];
        if declared {
            provides.push(COMPACTION_SIGNED_FLOOR_PROVIDES.to_owned());
        }
        assert_eq!(provides_signed_floor(&provides), declared);
        let floor = SignedFloor::new("entry-a");
        let conditions =
            conditions().with_signed_floor(floor.clone(), provides_signed_floor(&provides));
        let setup = setup().with_signed_floor(floor.clone(), provides_signed_floor(&provides));
        assert_eq!(conditions.signed_floor.as_ref(), declared.then_some(&floor));
        assert_eq!(setup.signed_floor.as_ref(), declared.then_some(&floor));
        for value in [
            serde_json::to_value(&conditions).unwrap(),
            serde_json::to_value(&setup).unwrap(),
        ] {
            assert_eq!(
                value.get("signed_floor"),
                declared.then_some(&json!({"through":"entry-a"}))
            );
        }
        let decoded: Conditions =
            serde_json::from_value(serde_json::to_value(conditions).unwrap()).unwrap();
        assert_eq!(decoded.signed_floor.as_ref(), declared.then_some(&floor));
        let decoded: SetupRequest =
            serde_json::from_value(serde_json::to_value(setup).unwrap()).unwrap();
        assert_eq!(decoded.signed_floor.as_ref(), declared.then_some(&floor));
    }
    assert!(!provides_signed_floor(&[
        "compaction-signed-floor/v2".into()
    ]));
}

#[test]
fn signed_floor_is_optional_and_decoders_remain_forward_compatible() {
    let mut conditions_json = serde_json::to_value(conditions()).unwrap();
    let mut setup_json = serde_json::to_value(setup()).unwrap();
    for value in [&mut conditions_json, &mut setup_json] {
        assert!(value.get("signed_floor").is_none());
        value["future_member"] = json!({"unknown":true});
    }
    assert!(
        serde_json::from_value::<Conditions>(conditions_json.clone())
            .unwrap()
            .signed_floor
            .is_none()
    );
    assert!(serde_json::from_value::<SetupRequest>(setup_json.clone())
        .unwrap()
        .signed_floor
        .is_none());
    for value in [&mut conditions_json, &mut setup_json] {
        value["signed_floor"] = json!({"through":"entry-a", "future_member":true});
    }
    assert_eq!(
        serde_json::from_value::<Conditions>(conditions_json)
            .unwrap()
            .signed_floor,
        Some(SignedFloor::new("entry-a"))
    );
    assert_eq!(
        serde_json::from_value::<SetupRequest>(setup_json)
            .unwrap()
            .signed_floor,
        Some(SignedFloor::new("entry-a"))
    );
}

#[test]
fn signed_floor_requires_a_string_entry_id() {
    for invalid in [json!({}), json!({"through":7}), json!({"through":null})] {
        assert!(serde_json::from_value::<SignedFloor>(invalid).is_err());
    }
    assert_eq!(
        serde_json::to_value(SignedFloor::new("entry-a")).unwrap(),
        json!({"through":"entry-a"})
    );
    assert_eq!(
        serde_json::from_value::<SignedFloor>(json!({"through":"entry-a"})).unwrap(),
        SignedFloor::new("entry-a")
    );
}
