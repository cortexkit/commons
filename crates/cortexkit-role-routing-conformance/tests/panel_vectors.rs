use cortexkit_role_routing_conformance::wire::panel::{
    check_panel_answer, check_panel_replacement, parse_panel_replacement_request,
    parse_select_panel_request, PanelProblem, PanelRejectedCandidate, PanelReplacement,
    PanelShortfallDetail, PanelState, SelectPanelAnswer,
};
use cortexkit_role_routing_conformance::wire::vectors;
use serde_json::{json, Value};

#[test]
fn routing_078_panel_detail_vectors() {
    let file: Value = serde_json::from_str(vectors::ROUTE_SELECT_PANEL_JSON).unwrap();
    let request = parse_select_panel_request(&file["requests"][0]["params"]).unwrap();
    let shortfall = file["answers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == "shortfall_typed_detail")
        .unwrap();
    let answer: SelectPanelAnswer = serde_json::from_value(shortfall["answer"].clone()).unwrap();
    assert_eq!(check_panel_answer(&request, &answer), Ok(()));
    let detail = answer.shortfall.unwrap().typed_detail().unwrap().unwrap();
    assert_eq!(json!(detail), shortfall["answer"]["shortfall"]["detail"]);
    assert_eq!(detail.causes.len(), 2);
    let fit = detail.causes[0].typed_detail().unwrap().unwrap();
    assert_eq!(fit.package_tokens, 10_000);
    assert_eq!(fit.budget_tokens, 6_000);
    assert_eq!(fit.context_window, 60_000);
    assert_eq!(detail.causes[1].reason, "auth_required");
    assert_eq!(
        detail.causes[1].detail.as_ref().unwrap()["code"],
        "auth_required"
    );

    for case in file["rejected_candidates"].as_array().unwrap() {
        let candidate: PanelRejectedCandidate =
            serde_json::from_value(case["candidate"].clone()).unwrap();
        assert_eq!(json!(candidate), case["candidate"]);
        assert!(candidate.typed_detail().unwrap().is_err());
    }

    let answer = serde_json::from_value(file["answers"][0]["answer"].clone()).unwrap();
    let state = PanelState::new(request, answer);
    for case in file["replacements"].as_array().unwrap() {
        let request = parse_panel_replacement_request(&case["params"]).unwrap();
        let answer: PanelReplacement = serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(json!(answer), case["answer"]);
        assert_eq!(check_panel_replacement(&state, &request, &answer), Ok(()));
        if case["name"] == "unseated_with_detail" {
            let detail: PanelShortfallDetail =
                serde_json::from_value(answer.detail.unwrap()).unwrap();
            assert_eq!(detail.causes[0].reason, "backoff_active");
            assert_eq!(detail.causes[0].reset_at_ms, Some(1_767_225_900_000));
        }
    }
    for case in file["invalid_replacements"].as_array().unwrap() {
        let request = parse_panel_replacement_request(&case["params"]).unwrap();
        let answer: PanelReplacement = serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(json!(answer), case["answer"]);
        assert_eq!(case["problem"], "SeatOrCause");
        assert_eq!(
            check_panel_replacement(&state, &request, &answer),
            Err(PanelProblem::SeatOrCause)
        );
    }
}
