use cortexkit_role_routing::{
    capabilities::{KNOWN, RUNNER_HARNESS_FACTS, SELECT_HARNESS_CAPABILITIES},
    declarations::{parse_consumer_declarations_serving, DeclarationError},
    errors::{NoRouteDetail, NoRouteReason, Refusal, RefusalCode},
    panel::parse_select_panel_request_serving,
    placement::HarnessCapability::{FinalText, Plans, SubmitTool},
    select::{parse_select_request, parse_select_request_serving, SelectAnswer, SelectRequest},
    vectors::ROUTE_SELECT_HARNESS_CAPABILITIES_JSON,
    ConsumerDeclarations,
};
use serde_json::{json, Value};

#[test]
fn harness_capability_vectors_decode_requests_declarations_panels_and_refusals() {
    let file: Value = serde_json::from_str(ROUTE_SELECT_HARNESS_CAPABILITIES_JSON).unwrap();
    assert_eq!(file["capabilities"], json!([SELECT_HARNESS_CAPABILITIES]));
    assert!(KNOWN.contains(&SELECT_HARNESS_CAPABILITIES));
    super::check_op::<SelectRequest, SelectAnswer>(
        "route-select-harness-capabilities.json",
        |params| parse_select_request_serving(params, &[SELECT_HARNESS_CAPABILITIES]),
    );

    for case in super::cases(&file, "refused_without_group") {
        let refusal = parse_select_request(&case["params"]).expect_err(super::name(case));
        assert_eq!(refusal.code.as_str(), case["refusal"]["code"]);
        assert_eq!(Value::Object(refusal.detail), case["refusal"]["detail"]);
    }

    // This is the existing declaration type, not a new capability-specific
    // decoder: the open set keeps the declaration readable across versions.
    let declaration: ConsumerDeclarations =
        serde_json::from_value(file["declaration"].clone()).unwrap();
    declaration
        .validate_serving(&[RUNNER_HARNESS_FACTS])
        .unwrap();
    let runner = declaration.runner.as_ref().unwrap();
    assert_eq!(
        runner
            .limits_for_harness(Some("alpha"))
            .unwrap()
            .capabilities,
        Some(vec![SubmitTool, FinalText, Plans])
    );
    assert_eq!(
        parse_consumer_declarations_serving(&file["declaration"], &[RUNNER_HARNESS_FACTS]).unwrap(),
        declaration
    );

    let panel = parse_select_panel_request_serving(
        &file["panel"]["params"],
        &[SELECT_HARNESS_CAPABILITIES],
    )
    .unwrap();
    let expected = Some(vec![SubmitTool, Plans]);
    assert_eq!(
        panel.requirements.as_ref().unwrap().harness_capabilities,
        expected
    );
    assert_eq!(
        panel.seat_request(&[]).requirements.harness_capabilities,
        expected
    );
    let refusal =
        cortexkit_role_routing::panel::parse_select_panel_request(&file["panel"]["params"])
            .unwrap_err();
    assert_eq!(
        refusal.code.as_str(),
        file["panel"]["without_group_refusal"]["code"]
    );
    assert_eq!(
        Value::Object(refusal.detail),
        file["panel"]["without_group_refusal"]["detail"]
    );

    for case in super::cases(&file, "refusals") {
        let refusal: Refusal = super::round_trip(super::name(case), &case["body"], &case["body"]);
        let detail: NoRouteDetail = refusal.detail_as().unwrap();
        assert_eq!(refusal.code, RefusalCode::NoRoute);
        assert_eq!(detail.reason, NoRouteReason::HarnessUnavailable);
        assert_eq!(
            refusal.wait_until_ms(),
            case["body"]["detail"]["reset_at_ms"].as_u64()
        );
        assert_eq!(refusal.detail["missing"], case["body"]["detail"]["missing"]);
    }
}

#[test]
fn panel_refuses_an_unknown_harness_capability_by_field_path() {
    let mut params: Value = serde_json::from_str(ROUTE_SELECT_HARNESS_CAPABILITIES_JSON).unwrap();
    let mut params = params["panel"]["params"].take();
    params["requirements"]["harness_capabilities"] = json!(["future_task"]);
    let refusal =
        parse_select_panel_request_serving(&params, &[SELECT_HARNESS_CAPABILITIES]).unwrap_err();
    assert_eq!(refusal.code, RefusalCode::InvalidRequirements);
    assert_eq!(refusal.detail["field"], "requirements.harness_capabilities");
}

#[test]
fn harness_capabilities_are_not_consumer_declaration_defaults() {
    let declaration = json!({"consumers": [{
        "name": "example_summary", "harness_capabilities": ["submit_tool"]
    }]});
    assert_eq!(
        parse_consumer_declarations_serving(&declaration, &[SELECT_HARNESS_CAPABILITIES]),
        Err(DeclarationError::UnknownMember {
            field: "consumers[0].harness_capabilities".into()
        })
    );
}
