use super::*;
use cortexkit_role_routing::{
    account_status, budget, consumer_models, describe, explain, head_choices, observe, panel,
    report, select,
};

const GROUP_FILES: &[&str] = &[
    "route-select-panel.json",
    "route-budget-quote.json",
    "route-explain.json",
    "route-account-status.json",
    "route-head-choices.json",
    "route-consumer-models.json",
    "route-report-usage-run-id.json",
    "route-select-pin-slot.json",
    "route-select-available-routes.json",
    "route-select-prefer-prompt-cache.json",
    "route-select-settings.json",
    "route-observe-backoff.json",
    "route-observe-credential-refused.json",
    "route-report-outcomes.json",
];

#[test]
fn new_group_vectors_describe_pairs() {
    for file in GROUP_FILES {
        let file = load(file);
        let group = file["group"].as_str().unwrap();
        assert!(capabilities::KNOWN.contains(&group), "{group}");
        if let Some(describes) = file["describes"].as_array() {
            for case in describes {
                let describe: RoleDescribe =
                    round_trip(name(case), &case["answer"], &case["answer"]);
                let expected = case.get("problem").map(|problem| {
                    assert_eq!(problem["kind"], "OpGroupMismatch");
                    assert_eq!(problem["group"], group);
                    let &(_, group) = capabilities::OP_GROUPS
                        .iter()
                        .find(|(_, g)| *g == group)
                        .unwrap();
                    describe::DescribeProblem::OpGroupMismatch { group }
                });
                assert_eq!(
                    check_describe(&describe),
                    expected.map_or(Ok(()), Err),
                    "{}",
                    name(case)
                );
            }
        }
    }
    assert_eq!(GROUP_FILES.len(), 14);
}

fn check_refusal(case: &Value, refusal: Refusal) {
    assert_eq!(
        json!(refusal.code),
        case["refusal"]["code"],
        "{}",
        name(case)
    );
    assert_eq!(
        json!(refusal.detail),
        case["refusal"]["detail"],
        "{}",
        name(case)
    );
}

fn legacy_members<Req, Ans>(file: &Value, parse: fn(&Value) -> Result<Req, Refusal>)
where
    Req: Serialize + Debug,
    Ans: DeserializeOwned + Serialize,
{
    for case in cases(file, "requests_without_group") {
        let parsed = parse(&case["params"]).unwrap();
        assert_eq!(json!(parsed), case["params"], "{}", name(case));
    }
    for case in cases(file, "answers_without_group") {
        let _: Ans = round_trip(name(case), &case["answer"], &case["answer"]);
    }
    for case in cases(file, "refused_without_group") {
        check_refusal(case, parse(&case["params"]).unwrap_err());
    }
}

#[test]
fn new_group_vectors_select_members() {
    for (file_name, group) in [
        ("route-select-pin-slot.json", capabilities::SELECT_PIN_SLOT),
        (
            "route-select-available-routes.json",
            capabilities::SELECT_AVAILABLE_ROUTES,
        ),
        (
            "route-select-prefer-prompt-cache.json",
            capabilities::SELECT_PREFER_PROMPT_CACHE,
        ),
        ("route-select-settings.json", capabilities::SELECT_SETTINGS),
    ] {
        let file = load(file_name);
        let parse = |params: &Value| parse_select_request_serving(params, &[group]);
        for case in cases(&file, "requests") {
            let request = parse(&case["params"]).unwrap();
            assert_eq!(json!(request), case["params"]);
            for answer in cases(&file, "answers") {
                let answer: SelectAnswer =
                    round_trip(name(answer), &answer["answer"], &answer["answer"]);
                assert_eq!(select::check_answer(&request, &answer), Ok(()));
            }
        }
        for case in cases(&file, "refused") {
            check_refusal(case, parse(&case["params"]).unwrap_err());
        }
        legacy_members::<SelectRequest, SelectAnswer>(&file, parse_select_request);
    }
}

#[test]
fn new_group_vectors_panel() {
    check_op::<panel::SelectPanelRequest, panel::SelectPanelAnswer>(
        "route-select-panel.json",
        panel::parse_select_panel_request,
    );
    let file = load("route-select-panel.json");
    let request = panel::parse_select_panel_request(&file["requests"][0]["params"]).unwrap();
    for case in cases(&file, "answers") {
        let answer: panel::SelectPanelAnswer =
            serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(panel::check_panel_answer(&request, &answer), Ok(()));
    }
    let answer = serde_json::from_value(file["answers"][0]["answer"].clone()).unwrap();
    let state = panel::PanelState::new(request, answer);
    for case in cases(&file, "replacements") {
        let request = panel::parse_panel_replacement_request(&case["params"]).unwrap();
        assert_eq!(json!(request), case["params"]);
        let answer: panel::PanelReplacement =
            round_trip(name(case), &case["answer"], &case["answer"]);
        assert_eq!(
            panel::check_panel_replacement(&state, &request, &answer),
            Ok(())
        );
    }
}

#[test]
fn new_group_vectors_inspection() {
    check_op::<budget::BudgetQuoteRequest, budget::BudgetQuote>(
        "route-budget-quote.json",
        budget::parse_budget_quote_request,
    );
    check_op::<explain::ExplainRequest, explain::ExplainAnswer>(
        "route-explain.json",
        explain::parse_explain_request,
    );
    check_op::<account_status::AccountStatusRequest, account_status::AccountStatusAnswer>(
        "route-account-status.json",
        account_status::parse_account_status_request,
    );
    check_op::<head_choices::HeadChoicesRequest, head_choices::HeadChoicesAnswer>(
        "route-head-choices.json",
        head_choices::parse_head_choices_request,
    );

    let file = load("route-budget-quote.json");
    let seats: Vec<budget::BudgetSeat> = file["capacities"]
        .as_array()
        .unwrap()
        .iter()
        .map(|seat| {
            budget::BudgetSeat::new(
                seat["decision_id"].as_str().unwrap(),
                seat["route_key"].as_str().unwrap(),
                seat["context_window"].as_u64().unwrap(),
                seat["max_output_tokens"].as_u64().unwrap(),
            )
        })
        .collect();
    for (request, answer) in cases(&file, "requests").iter().zip(cases(&file, "answers")) {
        let request = budget::parse_budget_quote_request(&request["params"]).unwrap();
        let answer = serde_json::from_value(answer["answer"].clone()).unwrap();
        assert_eq!(
            budget::check_budget_quote(&seats, request.prompt_tokens, &answer),
            Ok(())
        );
    }
    let file = load("route-explain.json");
    let selected = serde_json::from_value(file["select_answer"].clone()).unwrap();
    let request = explain::parse_explain_request(&file["requests"][0]["params"]).unwrap();
    let answer = serde_json::from_value(file["answers"][0]["answer"].clone()).unwrap();
    assert_eq!(explain::check_explain(&selected, &request, &answer), Ok(()));

    let file = load("route-account-status.json");
    let request =
        account_status::parse_account_status_request(&file["requests"][0]["params"]).unwrap();
    for case in cases(&file, "answers") {
        let answer = serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(
            account_status::check_account_status(&request, &answer),
            Ok(())
        );
    }
    let file = load("route-head-choices.json");
    let request = head_choices::parse_head_choices_request(&file["requests"][0]["params"]).unwrap();
    for case in cases(&file, "answers") {
        let answer = serde_json::from_value(case["answer"].clone()).unwrap();
        assert_eq!(head_choices::check_head_choices(&request, &answer), Ok(()));
    }
    for case in cases(&file, "invalid_answers") {
        assert!(
            serde_json::from_value::<head_choices::HeadChoicesAnswer>(case["answer"].clone())
                .is_err(),
            "{}",
            name(case)
        );
    }
}

#[test]
fn new_group_vectors_consumer_models() {
    use consumer_models::*;
    let file = load("route-consumer-models.json");
    let request = parse_consumer_models_request(&file["requests"][0]["params"]).unwrap();
    for case in cases(&file, "answers") {
        let answer: ConsumerModelsAnswer = round_trip(name(case), &case["answer"], &case["answer"]);
        assert_eq!(check_consumer_models(&request, &answer), Ok(()));
    }
    for case in cases(&file, "refused") {
        check_refusal(
            case,
            parse_consumer_models_request(&case["params"]).unwrap_err(),
        );
    }
    for case in cases(&file, "admission_refusals") {
        let _: Refusal = round_trip(name(case), &case["refusal"], &case["refusal"]);
        assert!(parse_consumer_models_request(&case["params"]).is_ok());
    }
    for case in cases(&file, "invalid_answers") {
        let decoded = serde_json::from_value::<ConsumerModelsAnswer>(case["answer"].clone());
        if case["decode_error"] == true {
            assert!(decoded.is_err(), "{}", name(case));
            continue;
        }
        let mut answer = decoded.unwrap();
        let expected = match case["problem"].as_str().unwrap() {
            "Consumer" => ConsumerModelsProblem::Consumer,
            "ModelsOrder" => ConsumerModelsProblem::ModelsOrder,
            field @ ("Elo" | "Eq") => {
                // JSON has no non-finite literals; the vector names the typed value.
                let index = case["non_finite"]["index"].as_u64().unwrap() as usize;
                let value = match case["non_finite"]["value"].as_str().unwrap() {
                    "Infinity" => f64::INFINITY,
                    "NaN" => f64::NAN,
                    value => panic!("unexpected non-finite value {value}"),
                };
                if field == "Elo" {
                    answer.models[index].elo = Some(value);
                    ConsumerModelsProblem::Elo { index }
                } else {
                    answer.models[index].eq = Some(value);
                    ConsumerModelsProblem::Eq { index }
                }
            }
            problem => panic!("unexpected problem {problem}"),
        };
        assert_eq!(
            check_consumer_models(&request, &answer),
            Err(expected),
            "{}",
            name(case)
        );
    }
}

#[test]
fn new_group_vectors_report_usage_run_id() {
    let file = load("route-report-usage-run-id.json");
    let parse = |params: &Value| {
        report::parse_report_request_serving(params, &[capabilities::REPORT_USAGE_RUN_ID])
    };
    for case in cases(&file, "requests") {
        let request = parse(&case["params"]).unwrap();
        assert_eq!(json!(request), case["params"]);
    }
    for case in cases(&file, "refused") {
        check_refusal(case, parse(&case["params"]).unwrap_err());
    }
    for case in cases(&file, "refused_without_group") {
        check_refusal(case, parse_report_request(&case["params"]).unwrap_err());
    }
}

#[test]
fn new_group_vectors_report_outcomes() {
    check_op::<ReportRequest, ReportAnswer>("route-report-outcomes.json", |params| {
        report::parse_report_request_serving(params, &[capabilities::REPORT_OUTCOMES])
    });
    let file = load("route-report-outcomes.json");
    legacy_members::<ReportRequest, ReportAnswer>(&file, parse_report_request);
    for case in cases(&file, "requests") {
        let request =
            report::parse_report_request_serving(&case["params"], &[capabilities::REPORT_OUTCOMES])
                .unwrap();
        assert!(request.outcome.is_known());
        assert!(!request.outcome.is_failure());
        assert_eq!(
            request.outcome.effect().affinity,
            report::AffinityEffect::Kept
        );
    }
    for case in cases(&file, "refusal_classes") {
        let name = case["name"].as_str().unwrap();
        let request =
            report::parse_report_request_serving(&case["params"], &[capabilities::REPORT_OUTCOMES])
                .unwrap_or_else(|refusal| panic!("{name}: {refusal}"));
        assert_eq!(
            json!(request.refusal_class),
            case["refusal_class"],
            "{name}"
        );
        assert_eq!(json!(request), case["params"], "{name}");
    }
    for case in cases(&file, "without_group") {
        let request = parse_report_request(&case["params"]).unwrap();
        assert!(!request.outcome.is_known());
        assert_eq!(
            json!(request.outcome.is_failure()),
            case["availability_penalty"]
        );
        assert_eq!(json!(request), case["params"]);
    }
}

#[test]
fn new_group_vectors_observe_backoff() {
    check_op::<AccountObserveRequest, AccountObserveAnswer>(
        "route-observe-backoff.json",
        |params| {
            observe::parse_account_observe_request_serving(params, &[capabilities::OBSERVE_BACKOFF])
        },
    );
    let file = load("route-observe-backoff.json");
    legacy_members::<AccountObserveRequest, AccountObserveAnswer>(
        &file,
        parse_account_observe_request,
    );
    for case in cases(&file, "without_group") {
        let request = parse_account_observe_request(&case["params"]).unwrap();
        assert!(!request.kind.is_known());
        assert_eq!(json!(request), case["params"]);
    }
    observe_authorization_vectors(&file, &[capabilities::OBSERVE_BACKOFF]);
}

#[test]
fn new_group_vectors_observe_credential_refused() {
    let groups = [
        capabilities::OBSERVE_BACKOFF,
        capabilities::OBSERVE_CREDENTIAL_REFUSED,
    ];
    check_op::<AccountObserveRequest, AccountObserveAnswer>(
        "route-observe-credential-refused.json",
        |params| {
            observe::parse_account_observe_request_serving(
                params,
                &[
                    capabilities::OBSERVE_BACKOFF,
                    capabilities::OBSERVE_CREDENTIAL_REFUSED,
                ],
            )
        },
    );
    let file = load("route-observe-credential-refused.json");
    for case in cases(&file, "without_group") {
        for groups in [&[][..], &[capabilities::OBSERVE_BACKOFF][..]] {
            let request =
                observe::parse_account_observe_request_serving(&case["params"], groups).unwrap();
            assert_eq!(
                request.kind,
                observe::ObserveKind::Other("credential_refused".into())
            );
            assert_eq!(json!(request), case["params"]);
        }
        assert_eq!(
            parse_account_observe_request(&case["params"]).unwrap().kind,
            observe::ObserveKind::Other("credential_refused".into())
        );
    }
    observe_authorization_vectors(&file, &groups);
}

fn observe_authorization_vectors(file: &Value, groups: &[&str]) {
    for case in cases(file, "authorization") {
        let request =
            observe::parse_account_observe_request_serving(&case["params"], groups).unwrap();
        let arm = match case["arm"].as_str().unwrap() {
            "runner" => observe::ObserveArm::Runner,
            "owner" => observe::ObserveArm::decision_owner(
                case["decision_id"].as_str().unwrap(),
                case["admission_id"].as_str().unwrap(),
                case["carried"].as_str(),
            ),
            "other" => observe::ObserveArm::Other,
            other => panic!("unknown arm {other}"),
        };
        let result = observe::check_observe_authorization(arm, &request);
        if case["allowed"] == true {
            assert_eq!(result, Ok(()), "{}", name(case));
        } else {
            assert_eq!(
                result.unwrap_err().code,
                RefusalCode::AccountNotAuthorized,
                "{}",
                name(case)
            );
        }
    }
}

#[test]
fn new_group_vectors_open_sets() {
    macro_rules! unknown {
        ($ty:ty, $case:expr) => {{
            let value = &$case["value"];
            let decoded: $ty = round_trip(name($case), value, value);
            assert!(!decoded.is_known(), "{}", name($case));
            assert_eq!(decoded.as_str(), value.as_str().unwrap());
        }};
    }
    for file in GROUP_FILES {
        for case in cases(&load(file), "unknown") {
            match case["type"].as_str().unwrap() {
                "PanelClass" => unknown!(panel::PanelClass, case),
                "PanelPhase" => unknown!(panel::PanelPhase, case),
                "ShortfallCode" => unknown!(panel::ShortfallCode, case),
                "ReplacementCause" => unknown!(panel::ReplacementCause, case),
                "OutputLimitStatus" => unknown!(budget::OutputLimitStatus, case),
                "OutputLimitReason" => unknown!(budget::OutputLimitReason, case),
                "ReasonCode" => unknown!(explain::ReasonCode, case),
                "LimitState" => unknown!(account_status::LimitState, case),
                "ProjectSettings" => unknown!(select::ProjectSettings, case),
                "Outcome" => unknown!(report::Outcome, case),
                "ObserveKind" => unknown!(observe::ObserveKind, case),
                other => panic!("unknown type {other}"),
            }
        }
    }
}

#[test]
fn new_group_vectors_refusal_bodies() {
    for file in GROUP_FILES {
        if let Some(refusals) = load(file)["refusals"].as_array() {
            for case in refusals {
                let refusal: Refusal = round_trip(name(case), &case["body"], &case["body"]);
                assert_eq!(refusal.wait_until_ms(), None);
            }
        }
    }
}
