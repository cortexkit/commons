use cortexkit_role_llm_runner::run::{RunResult, RunStatus};
use serde_json::{json, Value};

#[test]
fn run_result_preserves_finish() {
    for finish in [
        "stop",
        "length",
        "content_filter",
        "tool_calls",
        "error",
        "other",
        "future_reason",
    ] {
        let mut wire = serde_json::to_value(RunResult::new("r", "completed")).unwrap();
        wire["finish"] = json!(finish);
        let decoded: RunResult = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }
}

#[test]
fn run_status_preserves_finish() {
    for finish in [
        "stop",
        "length",
        "content_filter",
        "tool_calls",
        "error",
        "other",
        "future_reason",
    ] {
        let mut wire = serde_json::to_value(RunStatus::new("completed")).unwrap();
        wire["finish"] = json!(finish);
        let decoded: RunStatus = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), wire);
    }
}

#[test]
fn absent_finish_stays_absent() {
    let result: RunResult =
        serde_json::from_value(json!({"run_id":"r", "state":"completed"})).unwrap();
    let status: RunStatus = serde_json::from_value(json!({"state":"completed"})).unwrap();
    for wire in [
        serde_json::to_value(result).unwrap(),
        serde_json::to_value(status).unwrap(),
    ] {
        assert_eq!(wire.get("finish"), None::<&Value>);
    }
}

#[test]
fn finish_builders_encode_the_normalized_value() {
    use cortexkit_role_llm_runner::finish::Finish;
    for wire in [
        serde_json::to_value(RunResult::new("r", "completed").with_finish(Finish::Length)).unwrap(),
        serde_json::to_value(RunStatus::new("completed").with_finish(Finish::Length)).unwrap(),
    ] {
        assert_eq!(wire["finish"], "length");
    }
}

#[test]
fn finish_reason_requires_result_but_not_status_or_other_run_groups() {
    use cortexkit_role_llm_runner::{
        capabilities,
        describe::{check_describe, DescribeProblem, Major, RoleDescribe},
        ops,
    };
    let mut operations: Vec<String> = cortexkit_role_llm_runner::REQUIRED_OPS
        .iter()
        .map(|op| (*op).into())
        .collect();
    operations.push(ops::RUN_RESULT.into());
    let mut describe = RoleDescribe::new(
        vec![Major::new(
            cortexkit_role_llm_runner::PROVIDES,
            operations,
            "alpha",
        )],
        "test",
    )
    .with_capabilities(vec![capabilities::FINISH_REASON.into()])
    .with_session_capabilities_from("baseline");
    assert!(check_describe(&serde_json::to_value(&describe).unwrap()).is_ok());
    describe.majors[0].ops.retain(|op| op != ops::RUN_RESULT);
    let problems = check_describe(&serde_json::to_value(describe).unwrap()).unwrap_err();
    assert!(problems.iter().any(|problem| matches!(problem, DescribeProblem::GroupIncomplete { capability, op } if *capability == capabilities::FINISH_REASON && *op == ops::RUN_RESULT)));
}
