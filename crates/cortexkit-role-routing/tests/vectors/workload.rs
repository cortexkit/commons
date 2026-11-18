use cortexkit_role_routing::{
    capabilities::{CONSUMER_DECLARATIONS, KNOWN, SELECT_WORKLOAD},
    declarations::{parse_consumer_declarations_serving, ConsumerDeclarations, DeclarationError},
    errors::RefusalCode,
    select::{
        parse_select_request, parse_select_request_serving, parse_select_request_with_defaults,
        Requirements, SelectRequest, KNOWN_WORKLOADS,
    },
    vectors::ROUTE_SELECT_WORKLOAD_JSON,
    ConsumerDeclaration, DemandDefaults,
};
use serde_json::{json, Value};

#[test]
fn workload_vectors_preserve_names_and_apply_request_precedence() {
    let vectors: Value = serde_json::from_str(ROUTE_SELECT_WORKLOAD_JSON).unwrap();
    let groups = [SELECT_WORKLOAD];
    assert_eq!(vectors["capabilities"], json!(groups));
    assert!(KNOWN.contains(&SELECT_WORKLOAD));

    let declaration =
        parse_consumer_declarations_serving(&vectors["declaration"], &groups).unwrap();
    assert_eq!(
        serde_json::to_string(&declaration).unwrap(),
        vectors["declaration_wire"].as_str().unwrap(),
    );
    assert_eq!(
        declaration.consumers[0].defaults,
        DemandDefaults::new().with_workload("code"),
    );
    declaration.validate_serving(&groups).unwrap();

    for case in vectors["requests"].as_array().unwrap() {
        let params = &case["params"];
        let defaults = &declaration
            .consumers
            .iter()
            .find(|consumer| params["consumer"].as_str() == Some(consumer.name.as_str()))
            .unwrap()
            .defaults;
        let parsed = parse_select_request_with_defaults(params, &groups, defaults).unwrap();
        assert_eq!(
            parsed.requirements.workload.as_deref(),
            case["resolved_workload"].as_str(),
            "{}",
            case["name"],
        );
        if let Some(wire) = case["wire"].as_str() {
            assert_eq!(serde_json::to_string(&parsed).unwrap(), wire);
            assert_eq!(
                serde_json::to_string(&parse_select_request(params).unwrap()).unwrap(),
                wire
            );
        }
    }

    let case = &vectors["refused"];
    let refusal = parse_select_request_serving(&case["params"], &groups).unwrap_err();
    assert_eq!(
        json!({"code": refusal.code, "detail": refusal.detail}),
        case["refusal"]
    );
    let case = &vectors["invalid_declaration"];
    assert_eq!(
        parse_consumer_declarations_serving(&case["declaration"], &groups),
        Err(DeclarationError::InvalidField {
            field: case["field"].as_str().unwrap().into(),
        }),
    );
}

#[test]
fn workload_grammar_checks_only_shape_in_both_readers() {
    let groups = [SELECT_WORKLOAD];
    let longest = format!("a{}", "_".repeat(31));
    for name in KNOWN_WORKLOADS.iter().copied().chain(["a", &longest]) {
        let request =
            SelectRequest::new("example.review", Requirements::iq(50).with_workload(name));
        let params = serde_json::to_value(&request).unwrap();
        assert_eq!(
            parse_select_request_serving(&params, &groups).unwrap(),
            request
        );
        let consumer = ConsumerDeclaration::new("example.review")
            .with_defaults(DemandDefaults::new().with_workload(name));
        ConsumerDeclarations::new()
            .with_consumers(vec![consumer])
            .validate_serving(&groups)
            .unwrap();
    }

    for value in [
        json!(""),
        json!("Code"),
        json!("1code"),
        json!("_code"),
        json!("code-é"),
        json!("codé"),
        json!("code\n"),
        json!(format!("{longest}_")),
        json!(42),
        json!(["code"]),
    ] {
        let request =
            json!({"consumer": "example.review", "requirements": {"iq": 50, "workload": value}});
        let refusal = parse_select_request_serving(&request, &groups).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequirements);
        assert_eq!(refusal.detail["field"], "workload");
        let declaration = json!({"consumers": [{"name": "example.review", "workload": value}]});
        assert_eq!(
            parse_consumer_declarations_serving(&declaration, &groups),
            Err(DeclarationError::InvalidField {
                field: "consumers[0].workload".into()
            }),
        );
    }
}

#[test]
fn workload_is_gated_independently_of_other_declaration_defaults() {
    let other_groups = [CONSUMER_DECLARATIONS];
    let only_workload = [SELECT_WORKLOAD];
    let absent = json!({"consumers": [{"name": "example.review"}]});
    for value in [json!("code"), Value::Null] {
        let request =
            json!({"consumer": "example.review", "requirements": {"iq": 50, "workload": value}});
        let refusal = parse_select_request_serving(&request, &other_groups).unwrap_err();
        assert_eq!(refusal.code, RefusalCode::InvalidRequirements);
        assert_eq!(refusal.detail["field"], "workload");

        let mut declaration = absent.clone();
        declaration["consumers"][0]["workload"] = value.clone();
        let expected = DeclarationError::UnknownMember {
            field: "consumers[0].workload".into(),
        };
        let decoded: ConsumerDeclarations = serde_json::from_value(declaration.clone()).unwrap();
        assert_eq!(
            decoded.validate_serving(&other_groups).unwrap_err(),
            expected
        );
        assert_eq!(
            parse_consumer_declarations_serving(&declaration, &other_groups).unwrap_err(),
            expected
        );
        decoded.validate_serving(&only_workload).unwrap();
        if value.is_null() {
            assert_eq!(serde_json::to_value(&decoded).unwrap(), absent);
        }
    }

    let defaults = DemandDefaults::new().with_iq(80).with_workload("code");
    let request = json!({"consumer": "example.review", "requirements": {}});
    let parsed = parse_select_request_with_defaults(&request, &other_groups, &defaults).unwrap();
    assert_eq!(parsed.requirements, Requirements::iq(80));
    let refusal =
        parse_select_request_with_defaults(&request, &only_workload, &defaults).unwrap_err();
    assert_eq!(refusal.detail["field"], "iq");
}
