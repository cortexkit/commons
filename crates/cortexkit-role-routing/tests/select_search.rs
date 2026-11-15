use cortexkit_role_routing::{
    capabilities, parse_search_declarations,
    select_engine::Privacy,
    select_media::{parse_select_media_request, SelectMediaAnswer},
    select_search::{parse_select_search_request, SearchDeclarationsError},
};
use serde_json::{json, Value};

fn request() -> Value {
    json!({"consumer":"example-search.web_search","kind":"search","requirements":{},"privacy":"remote_allowed"})
}

fn parse(
    value: &Value,
) -> Result<
    cortexkit_role_routing::select_search::SelectSearchRequest,
    cortexkit_role_routing::errors::Refusal,
> {
    parse_select_search_request(value, capabilities::KNOWN, &["search"])
}

fn field(value: &Value, code: &str, field: &str) {
    let refusal = parse(value).unwrap_err();
    assert_eq!(serde_json::to_value(refusal.code).unwrap(), code);
    assert_eq!(refusal.detail["field"], field);
}

#[test]
fn search_declarations_are_atomic_and_count_unicode_scalars() {
    let route = json!({"provider":"provider","model":"model"});
    let valid = json!({"routes":[route.clone()]});
    assert_eq!(parse_search_declarations(&valid).unwrap().routes.len(), 1);
    for bad in [
        json!({}),
        json!({"routes":null}),
        json!({"routes":[],"account":"hidden"}),
        json!({"routes":[route.clone(),{"provider":"provider","model":""}]}),
        json!({"routes":[route.clone(),{"provider":"provider","model":"x","extra":1}]}),
        json!({"routes":[{"provider":"🙂".repeat(257),"model":"x"}]}),
    ] {
        assert!(parse_search_declarations(&bad).is_err(), "{bad}");
    }
    let error = parse_search_declarations(&json!({"routes":[route.clone(),route]})).unwrap_err();
    assert_eq!(error, SearchDeclarationsError::DuplicateRoute { index: 1 });
    assert_eq!(error.code(), "duplicate_route");
    assert!(parse_search_declarations(
        &json!({"routes":[{"provider":"🙂".repeat(256),"model":"é".repeat(256)}]})
    )
    .is_ok());
}

#[test]
fn search_request_dispatch_is_advertised_and_separate() {
    let value = request();
    assert_eq!(serde_json::to_value(parse(&value).unwrap()).unwrap(), value);
    for absent in ["select_search", "select_kind", "select_media"] {
        let groups: Vec<_> = capabilities::KNOWN
            .iter()
            .copied()
            .filter(|g| *g != absent)
            .collect();
        assert_eq!(
            parse_select_search_request(&value, &groups, &["search"])
                .unwrap_err()
                .detail["field"],
            "kind"
        );
    }
    assert!(parse_select_search_request(&value, capabilities::KNOWN, &["transcribe"]).is_err());
    assert!(parse_select_media_request(&value, capabilities::KNOWN, &["search"]).is_err());
    let mut llm = value.clone();
    llm.as_object_mut().unwrap().remove("privacy");
    llm["kind"] = json!("llm");
    assert!(
        cortexkit_role_routing::select::parse_select_request_serving(&llm, capabilities::KNOWN)
            .is_err()
    );
}

#[test]
fn search_describe_requires_both_capability_dependencies_by_name() {
    use cortexkit_role_routing::describe::{check_describe, DescribeProblem, RoleDescribe};
    for (groups, missing) in [
        (vec![capabilities::SELECT_SEARCH], capabilities::SELECT_KIND),
        (
            vec![capabilities::SELECT_SEARCH, capabilities::SELECT_KIND],
            capabilities::SELECT_MEDIA,
        ),
    ] {
        let mut describe = RoleDescribe::new(vec![], false);
        for group in groups {
            describe = describe.with_capability(group);
        }
        assert_eq!(
            check_describe(&describe),
            Err(DescribeProblem::MissingCapabilityDependency {
                group: capabilities::SELECT_SEARCH,
                requires: missing,
            })
        );
    }
}

#[test]
fn search_refuses_every_other_requirement_by_name() {
    for name in cortexkit_role_routing::select::REQUIREMENT_FIELDS
        .iter()
        .chain(cortexkit_role_routing::select_media::REQUIREMENT_FIELDS)
        .chain(["future"].iter())
    {
        if *name == "iq" {
            continue;
        }
        let mut value = request();
        value["requirements"][*name] = json!(null);
        field(&value, "invalid_requirements", name);
    }
    for iq in [json!(101), json!(-1), json!(1.5), json!("50")] {
        let mut value = request();
        value["requirements"]["iq"] = iq;
        field(&value, "invalid_requirements", "iq");
    }
    for iq in [json!(0), json!(100), json!(null)] {
        let mut value = request();
        value["requirements"]["iq"] = iq;
        assert!(parse(&value).is_ok());
    }
}

#[test]
fn search_privacy_and_caller_local_are_strict() {
    let mut value = request();
    value.as_object_mut().unwrap().remove("privacy");
    field(&value, "invalid_request", "privacy");
    value["privacy"] = json!("local_only");
    assert_eq!(parse(&value).unwrap().privacy, Privacy::LocalOnly);
    value["privacy"] = json!("future");
    field(&value, "invalid_request", "privacy");
    value["privacy"] = json!("remote_allowed");
    value["caller_local"] = json!([]);
    field(&value, "invalid_request", "caller_local");
}

#[test]
fn search_allowance_requires_eligible_membership_and_is_search_only() {
    let mut value = request();
    let route = json!({"provider":"provider","model":"model"});
    value["allowance_left"] = json!([route.clone()]);
    field(&value, "invalid_request", "allowance_left[0]");
    value["eligible_routes"] = json!([]);
    field(&value, "invalid_request", "allowance_left[0]");
    value["eligible_routes"] = json!([route.clone()]);
    value["allowance_left"] = json!([route.clone(), route]);
    assert_eq!(parse(&value).unwrap().allowance_left.unwrap().len(), 2);
    value["allowance_left"][1]["extra"] = json!(true);
    field(&value, "invalid_request", "allowance_left[1]");
    for kind in ["llm", "transcribe"] {
        value["kind"] = json!(kind);
        value["requirements"] = json!({"iq":0});
        let refusal = if kind == "llm" {
            value.as_object_mut().unwrap().remove("privacy");
            cortexkit_role_routing::select::parse_select_request_serving(
                &value,
                capabilities::KNOWN,
            )
            .unwrap_err()
        } else {
            value["privacy"] = json!("remote_allowed");
            parse_select_media_request(&value, capabilities::KNOWN, &[kind]).unwrap_err()
        };
        assert_eq!(refusal.detail["field"], "allowance_left");
    }
}

#[test]
fn search_answer_reuses_unit_price_and_remote_module_matrix() {
    let request = parse(&request()).unwrap();
    let value = json!({"decision_id":"search-1","expires_at_ms":600000,
        "engine":{"provider":"provider","model":"model","module":"example-search","local":false,"caller_local":false},
        "price":{"basis":"metered","rates":{"request.per_request":{"rate":0.01}}}});
    let answer: SelectMediaAnswer = serde_json::from_value(value.clone()).unwrap();
    cortexkit_role_routing::select_search::check_answer(&request, &answer).unwrap();
    assert_eq!(serde_json::to_value(&answer).unwrap(), value);
    for key in ["local", "caller_local"] {
        let mut bad = value.clone();
        bad["engine"][key] = json!(true);
        assert!(cortexkit_role_routing::select_search::check_answer(
            &request,
            &serde_json::from_value(bad).unwrap()
        )
        .is_err());
    }
}
