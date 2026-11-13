use cortexkit_role_routing::panel::parse_select_panel_request_serving;
use serde_json::json;

#[test]
fn panel_estimate_answer_has_no_ids_and_ordinary_answers_require_them() {
    use cortexkit_role_routing::panel::{SelectPanelAnswer, SelectPanelEstimate};
    let wire = json!({
        "policy":{"size":2,"top":1,"extra":1},
        "seats":[{"slot":0,"phase":"top","family":"example","answer":{
            "route":{"provider":"example","model":"large"},
            "runner":{"module":"example-runner","model":{"provider":"example","model":"large"}},
            "context_window":200_000,"max_output_tokens":32_000,
            "expires_at_ms":30_000,"explanation":"Adequate capacity."
        }}],
        "shortfall":{"code":"package_unmeetable","detail":{
            "message":"One candidate is too small.","package_tokens":70_000,
            "causes":[{"model":"example/small","reason":"package_unmeetable","detail":{
                "package_tokens":70_000,"budget_tokens":40_000,"context_window":128_000,
                "context_window_known":true,"catalog_max_output_tokens":32_000,
                "output_reserve_tokens":32_000,"prompt_reserve_tokens":16_000
            }}]
        }}
    });
    let estimate: SelectPanelEstimate = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(estimate).unwrap(), wire);
    assert!(serde_json::from_value::<SelectPanelAnswer>(wire.clone()).is_err());
    let mut ordinary = wire.clone();
    ordinary["panel_id"] = json!("panel");
    assert!(serde_json::from_value::<SelectPanelAnswer>(ordinary.clone()).is_err());
    ordinary["seats"][0]["answer"]["decision_id"] = json!("decision");
    assert!(serde_json::from_value::<SelectPanelAnswer>(ordinary.clone()).is_ok());
    assert!(serde_json::from_value::<SelectPanelEstimate>(ordinary).is_err());
}

#[test]
fn panel_estimate_request_is_reader_first() {
    for estimate in [json!(true), json!(false), serde_json::Value::Null] {
        let wire = json!({"class":"audit","consumer":"example.task","estimate":estimate});
        let refusal = parse_select_panel_request_serving(&wire, &["select_panel"]).unwrap_err();
        assert_eq!(refusal.detail["field"], "estimate");
        let parsed =
            parse_select_panel_request_serving(&wire, &["select_panel", "select_panel_estimate"])
                .unwrap();
        let encoded = serde_json::to_value(parsed).unwrap();
        if !estimate.is_null() {
            assert_eq!(encoded["estimate"], estimate);
        } else {
            assert!(encoded.get("estimate").is_none());
        }
    }
}

#[test]
fn panel_estimate_invalid_flag_is_refused_by_name() {
    let wire = json!({"class":"audit","consumer":"example.task","estimate":"yes"});
    let refusal =
        parse_select_panel_request_serving(&wire, &["select_panel", "select_panel_estimate"])
            .unwrap_err();
    assert_eq!(refusal.detail["field"], "estimate");
}
