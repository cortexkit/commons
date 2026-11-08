use cortexkit_role_compaction::rules::{EditCost, ProviderRules, SignedParts};
use cortexkit_role_compaction::setup::{ModelRef, SetupRequest};
use cortexkit_role_compaction::view::{AppliedViewAnswer, View};
use serde_json::json;

#[test]
fn view_anchor_setup_gates_sealed_bytes_and_tracks_the_latest_setup() {
    let base = SetupRequest::new(
        "example",
        ModelRef::new("example-provider", "example-model"),
        32768,
        4096,
        ProviderRules::new(SignedParts::Bound, EditCost::Suffix),
        vec![],
        1,
    );
    let cases: serde_json::Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/view-anchor-shapes.json"
    ))
    .unwrap();
    for case in cases["cases"].as_array().unwrap() {
        let mut wire = serde_json::to_value(&base).unwrap();
        wire.as_object_mut()
            .unwrap()
            .extend(case["setup"].as_object().unwrap().clone());
        let setup: SetupRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&setup).unwrap(), wire);
        let view = View::seal_for_setup(
            &setup,
            1,
            vec![],
            Some("anchor".into()),
            Some("strip".into()),
        );
        assert_eq!(
            serde_json::to_string(&view).unwrap(),
            case["sealed"].as_str().unwrap(),
            "{}",
            case["name"]
        );
        assert!(view.verify());
    }
    let seal = |setup: &SetupRequest| {
        View::seal_for_setup(
            setup,
            2,
            vec![],
            Some("new-anchor".into()),
            Some("strip".into()),
        )
    };
    let enabled = base.clone().with_view_anchor(true);
    assert_eq!(seal(&enabled).built_through.as_deref(), Some("new-anchor"));
    let removed = base;
    assert_eq!(seal(&removed), View::seal(2, vec![]));
    assert_eq!(
        seal(&enabled.with_view_anchor(false)),
        View::seal(2, vec![])
    );
}

#[test]
fn applied_by_is_optional_and_answers_still_ignore_unknown_fields() {
    let old: AppliedViewAnswer = serde_json::from_value(json!({"future": true})).unwrap();
    assert_eq!(old.applied_by, None);
    assert_eq!(serde_json::to_value(old).unwrap(), json!({}));
    let answer = AppliedViewAnswer::new()
        .with_view(View::seal(1, vec![]))
        .with_applied_by("example-provider");
    let mut wire = serde_json::to_value(&answer).unwrap();
    assert_eq!(wire["applied_by"], "example-provider");
    wire["future"] = json!(true);
    assert_eq!(
        serde_json::from_value::<AppliedViewAnswer>(wire).unwrap(),
        answer
    );
}

#[test]
fn child_harness_is_optional_and_round_trips_without_identity_inference() {
    let setup = SetupRequest::new(
        "example",
        ModelRef::new("example-model-provider", "example-model"),
        32768,
        4096,
        ProviderRules::new(SignedParts::Bound, EditCost::Suffix),
        vec![],
        1,
    );
    let old = serde_json::to_value(&setup).unwrap();
    assert!(old.get("child_harness").is_none());
    assert_eq!(
        serde_json::from_value::<SetupRequest>(old)
            .unwrap()
            .child_harness,
        None
    );
    let setup = setup.with_child_harness("example-child-host");
    let wire = serde_json::to_value(&setup).unwrap();
    assert_eq!(wire["child_harness"], "example-child-host");
    assert_eq!(serde_json::from_value::<SetupRequest>(wire).unwrap(), setup);
}

#[test]
fn policy_members_enter_the_view_checksum_only_when_present() {
    let old = View::seal(1, vec![]);
    assert_eq!(
        old.checksum,
        cortexkit_role_compaction::digest::checksum(&json!({"version": 1, "edits": []}))
    );
    assert!(serde_json::to_value(&old)
        .unwrap()
        .get("built_through")
        .is_none());
    assert!(serde_json::to_value(&old)
        .unwrap()
        .get("new_signed")
        .is_none());
    for (anchor, policy) in [
        (Some("anchor"), None),
        (None, Some("strip")),
        (Some("anchor"), Some("strip")),
        (Some("anchor"), Some("unknown-policy")),
    ] {
        let view = View::seal_with_policy(
            1,
            vec![],
            anchor.map(str::to_owned),
            policy.map(str::to_owned),
        );
        let mut content = json!({"version": 1, "edits": []});
        if let Some(anchor) = anchor {
            content["built_through"] = json!(anchor);
        }
        if let Some(policy) = policy {
            content["new_signed"] = json!(policy);
        }
        assert_eq!(
            view.checksum,
            cortexkit_role_compaction::digest::checksum(&content)
        );
        assert_ne!(view.checksum, old.checksum);
        assert!(view.verify());
        assert_eq!(
            serde_json::from_value::<View>(serde_json::to_value(&view).unwrap()).unwrap(),
            view
        );
        let mut changed = view.clone();
        changed.built_through = Some("different-message".into());
        assert!(!changed.verify());
        let mut changed = view;
        changed.new_signed = Some("different-policy".into());
        assert!(!changed.verify());
    }
}
