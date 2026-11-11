use cortexkit_role_compaction::{
    notice::Conditions,
    provides_limits,
    read::{Head, ReadName, ReadPage},
    ride::{RideCause, RideRequest},
    setup::{Limits, PageLimits, SetupRequest, WindowLimits},
    wire::{self, Refusal},
    COMPACTION_LIMITS_PROVIDES, COMPACTION_PROVIDES,
};
use serde_json::{json, Value};

#[test]
fn limits_setup_round_trip() {
    let mut setup: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/setup-request.json"
    ))
    .unwrap();
    setup["params"]["limits"] = json!({
        "page": {"max_entries": 512, "max_bytes": 1_048_576},
        "select_max": 256,
        "window": {"entries": 8_000, "bytes": 67_108_864}
    });
    let decoded: SetupRequest = serde_json::from_value(setup["params"].clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), setup["params"]);
}

#[test]
fn limits_window_over_budget_round_trip() {
    let notice: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/notice-request.json"
    ))
    .unwrap();
    let mut conditions = notice["params"]["conditions"].clone();
    conditions["window_over_budget"] = json!(true);
    let decoded: Conditions = serde_json::from_value(conditions.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), conditions);
}

#[test]
fn limits_unanswered_round_trip() {
    for names in [json!(["message-b", "message-a"]), json!([2, 1])] {
        let page = json!({"entries": [], "head": {}, "unanswered": names});
        let decoded: ReadPage = serde_json::from_value(page.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), page);
    }
}

#[test]
fn limits_budget_ride_does_not_imply_cache_loss() {
    let notice: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/notice-request.json"
    ))
    .unwrap();
    let conditions: Conditions =
        serde_json::from_value(notice["params"]["conditions"].clone()).unwrap();
    let cause: RideCause = serde_json::from_value(json!("window_over_budget")).unwrap();
    assert_eq!(cause, RideCause::WindowOverBudget);
    for causes in [
        vec![cause.clone()],
        vec![cause.clone(), RideCause::OverLimit],
    ] {
        assert!(!RideRequest::new(causes, 1_000, conditions.clone()).cache_lost());
    }
    assert!(RideRequest::new(vec![cause, RideCause::CacheExpired], 1_000, conditions).cache_lost());
}

fn setup() -> SetupRequest {
    let request: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/setup-request.json"
    ))
    .unwrap();
    wire::params(request["params"].clone()).unwrap()
}

fn conditions() -> Conditions {
    let request: Value = serde_json::from_str(include_str!(
        "../test-vectors/compaction-v2/notice-request.json"
    ))
    .unwrap();
    wire::params(request["params"]["conditions"].clone()).unwrap()
}

#[test]
fn limits_builders_and_each_optional_bound_round_trip() {
    let bounds = Limits::new()
        .with_page(
            PageLimits::new()
                .with_max_entries(512)
                .with_max_bytes(1_048_576),
        )
        .with_select_max(256)
        .with_window(
            WindowLimits::new()
                .with_entries(8_000)
                .with_bytes(67_108_864),
        );
    assert_eq!(
        serde_json::to_value(&bounds).unwrap(),
        json!({
            "page": {"max_entries": 512, "max_bytes": 1_048_576},
            "select_max": 256,
            "window": {"entries": 8_000, "bytes": 67_108_864}
        })
    );
    let setup = setup().with_limits(bounds.clone());
    assert_eq!(setup.limits, Some(bounds));
    assert_eq!(
        wire::params::<SetupRequest>(serde_json::to_value(&setup).unwrap()).unwrap(),
        setup
    );

    for value in [
        json!({}),
        json!({"page": {}}),
        json!({"page": {"max_entries": 1}}),
        json!({"page": {"max_bytes": 0}}),
        json!({"select_max": 0}),
        json!({"window": {}}),
        json!({"window": {"entries": 0}}),
        json!({"window": {"bytes": 0}}),
    ] {
        let decoded: Limits = wire::params(value.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), value);
    }
    assert_eq!(Limits::new(), Limits::default());
    assert_eq!(PageLimits::new(), PageLimits::default());
    assert_eq!(WindowLimits::new(), WindowLimits::default());
}

#[test]
fn limits_validate_integer_bounds_and_report_nested_field_paths() {
    for (pointer, field) in [
        ("/limits/page/max_entries", "limits.page.max_entries"),
        ("/limits/page/max_bytes", "limits.page.max_bytes"),
        ("/limits/select_max", "limits.select_max"),
        ("/limits/window/entries", "limits.window.entries"),
        ("/limits/window/bytes", "limits.window.bytes"),
    ] {
        let mut params = serde_json::to_value(setup()).unwrap();
        params["limits"] = json!({
            "page": {"max_entries": 1, "max_bytes": 0},
            "select_max": 0, "window": {"entries": 0, "bytes": 0}
        });
        *params.pointer_mut(pointer).unwrap() = json!(9_007_199_254_740_991_u64);
        let decoded: SetupRequest = wire::params(params.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), params);
        for invalid in [
            json!(-1),
            json!(1.5),
            json!("512"),
            json!(true),
            json!({}),
            json!(9_007_199_254_740_992_u64),
            json!(u64::MAX),
        ] {
            *params.pointer_mut(pointer).unwrap() = invalid.clone();
            let refusal = wire::params::<SetupRequest>(params.clone()).unwrap_err();
            assert_eq!(
                refusal.code,
                wire::codes::INVALID_PARAMS,
                "{field}: {invalid}"
            );
            assert_eq!(refusal.detail, Some(json!({"field": field})), "{invalid}");
        }
    }
    let mut params = serde_json::to_value(setup()).unwrap();
    params["limits"] = json!({"page": {"max_entries": 0}});
    let refusal = wire::params::<SetupRequest>(params).unwrap_err();
    assert_eq!(
        refusal.detail,
        Some(json!({"field": "limits.page.max_entries"}))
    );
}

#[test]
fn limits_optional_members_follow_existing_decode_conventions() {
    let mut params = serde_json::to_value(setup()).unwrap();
    assert!(params.get("limits").is_none());
    params["limits"] = Value::Null;
    assert_eq!(
        wire::params::<SetupRequest>(params.clone()).unwrap(),
        setup()
    );
    params["limits"] = json!({
        "page": {"max_entries": null, "max_bytes": null, "future": true},
        "select_max": null,
        "window": {"entries": null, "bytes": null, "future": true},
        "future": true
    });
    let decoded: SetupRequest = wire::params(params.clone()).unwrap();
    assert_eq!(
        serde_json::to_value(decoded).unwrap()["limits"],
        json!({"page": {}, "window": {}})
    );
    for invalid in [json!(false), json!(1), json!([true]), json!("512")] {
        params["limits"] = invalid.clone();
        assert!(wire::params::<SetupRequest>(params.clone()).is_err());
        params["limits"] = json!({"page": invalid.clone()});
        assert!(wire::params::<SetupRequest>(params.clone()).is_err());
        params["limits"] = json!({"window": invalid});
        assert!(wire::params::<SetupRequest>(params.clone()).is_err());
    }
    let mut conditions = serde_json::to_value(conditions()).unwrap();
    for invalid in [json!(1), json!("true"), json!([]), json!({})] {
        conditions["window_over_budget"] = invalid;
        assert_eq!(
            wire::params::<Conditions>(conditions.clone())
                .unwrap_err()
                .detail,
            Some(json!({"field": "window_over_budget"}))
        );
    }
    conditions["window_over_budget"] = Value::Null;
    assert!(wire::params::<Conditions>(conditions)
        .unwrap()
        .window_over_budget
        .is_none());
}

#[test]
fn limits_unanswered_validates_name_types_and_tag_bounds() {
    for names in [
        json!([]),
        json!([0, 9_007_199_254_740_991_u64]),
        json!(["message-a", 42]),
    ] {
        let page = json!({"entries": [], "head": {}, "unanswered": names});
        let decoded: ReadPage = wire::params(page.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), page);
    }
    for invalid in [
        json!(-1),
        json!(1.5),
        json!(true),
        json!(null),
        json!({}),
        json!([]),
        json!(9_007_199_254_740_992_u64),
    ] {
        let refusal = wire::params::<ReadPage>(json!({
            "entries": [], "head": {}, "unanswered": [invalid]
        }))
        .unwrap_err();
        assert_eq!(refusal.code, wire::codes::INVALID_PARAMS);
        assert_eq!(refusal.detail, Some(json!({"field": "unanswered[0]"})));
    }
    let page: ReadPage =
        wire::params(json!({"entries": [], "head": {}, "unanswered": null})).unwrap();
    assert!(page.unanswered.is_none());
}

#[test]
fn limits_capability_projection_omits_members_and_causes_for_legacy_providers() {
    assert_eq!(COMPACTION_LIMITS_PROVIDES, "compaction-limits/v1");
    for declared in [false, true] {
        let mut provides = vec![COMPACTION_PROVIDES.to_owned()];
        if declared {
            provides.push(COMPACTION_LIMITS_PROVIDES.to_owned());
        }
        assert_eq!(provides_limits(&provides), declared);
        let conditions = conditions().with_window_over_budget(true, provides_limits(&provides));
        assert_eq!(conditions.window_over_budget, declared.then_some(true));
        assert_eq!(
            serde_json::to_value(&conditions)
                .unwrap()
                .get("window_over_budget"),
            declared.then_some(&json!(true))
        );
        assert!(conditions
            .clone()
            .with_window_over_budget(true, false)
            .window_over_budget
            .is_none());
        assert!(conditions
            .clone()
            .with_window_over_budget(false, true)
            .window_over_budget
            .is_none());

        for name in [ReadName::Mid("message-a".into()), ReadName::Tag(42)] {
            let page =
                ReadPage::new(vec![], Head::new()).with_unanswered(vec![name.clone()], declared);
            assert_eq!(page.unanswered, declared.then_some(vec![name]));
            assert_eq!(
                serde_json::to_value(&page)
                    .unwrap()
                    .get("unanswered")
                    .is_some(),
                declared
            );
            assert!(page
                .clone()
                .with_unanswered(vec![ReadName::Tag(43)], false)
                .unanswered
                .is_none());
            assert!(page.with_unanswered(vec![], true).unanswered.is_none());
        }

        let setup = setup()
            .with_window_over_budget_detection(true, true)
            .with_window_over_budget_detection(true, declared);
        assert_eq!(
            setup.detects.contains(&RideCause::WindowOverBudget),
            declared
        );
        assert_eq!(
            serde_json::to_value(&setup).unwrap()["detects"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|cause| **cause == json!("window_over_budget"))
                .count(),
            usize::from(declared)
        );
        let ride = RideRequest::new(vec![RideCause::OverLimit], 1_000, conditions)
            .with_window_over_budget(true, true)
            .with_window_over_budget(true, declared);
        assert_eq!(ride.has(RideCause::WindowOverBudget), declared);
        assert_eq!(
            serde_json::to_value(&ride).unwrap()["causes"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|cause| **cause == json!("window_over_budget"))
                .count(),
            usize::from(declared)
        );
        assert!(!ride
            .with_window_over_budget(false, true)
            .has(RideCause::WindowOverBudget));
    }
    for other in [
        "compaction-limits/v2",
        "compaction-limits/v1-extra",
        "compaction-signed-floor/v1",
    ] {
        assert!(!provides_limits(&[other.into()]));
    }
}

#[test]
fn limits_select_too_many_refusal_has_machine_readable_detail() {
    let refusal = Refusal::select_too_many(256);
    let encoded = serde_json::to_value(&refusal).unwrap();
    assert_eq!(
        encoded,
        json!({
            "code": "invalid_params", "message": "too many names in selection",
            "detail": {"field": "select", "problem": "too_many", "max": 256}
        })
    );
    assert_eq!(wire::params::<Refusal>(encoded).unwrap(), refusal);
}
