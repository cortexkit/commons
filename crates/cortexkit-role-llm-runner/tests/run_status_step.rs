use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, DescribeProblem, Major, RoleDescribe},
    ops,
    run::{RunStatus, RunStatusStep, RunStatusStepPhase as Phase},
    PROVIDES, REQUIRED_OPS,
};

#[test]
fn step_liveness_requires_run_status_group_and_operation() {
    let mut operations: Vec<String> = REQUIRED_OPS.iter().map(|op| (*op).into()).collect();
    operations.push(ops::RUN_STATUS.into());
    let mut describe = RoleDescribe::new(vec![Major::new(PROVIDES, operations, "alpha")], "test")
        .with_capabilities(vec![capabilities::STEP_LIVENESS.into()])
        .with_session_capabilities_from("baseline");
    let problems = check_describe(&serde_json::to_value(&describe).unwrap()).unwrap_err();
    assert!(problems.iter().any(|problem| matches!(problem, DescribeProblem::Undecodable(message) if message == "step_liveness requires run_status")));

    describe.capabilities.push(capabilities::RUN_STATUS.into());
    assert!(check_describe(&serde_json::to_value(&describe).unwrap()).is_ok());
    describe.majors[0].ops.retain(|op| op != ops::RUN_STATUS);
    let problems = check_describe(&serde_json::to_value(describe).unwrap()).unwrap_err();
    assert!(problems.iter().any(|problem| matches!(problem, DescribeProblem::GroupIncomplete { capability, op } if *capability == capabilities::STEP_LIVENESS && *op == ops::RUN_STATUS)));
}

#[test]
fn run_status_step_vector_round_trips() {
    let bytes = include_str!("../test-vectors/llm-runner-v1/run-status-step.json");
    let statuses: Vec<RunStatus> = serde_json::from_str(bytes).unwrap();
    let active = RunStatus::new("active");
    assert_eq!(
        statuses,
        [
            active.clone().with_step(
                RunStatusStep::new(Phase::AwaitingResponse)
                    .with_idle_deadline_ms(1_700_000_030_000)
                    .with_content_deadline_ms(1_700_000_120_000)
            ),
            active.clone().with_step(
                RunStatusStep::new(Phase::Streaming)
                    .with_last_event_at_ms(1_700_000_001_000)
                    .with_idle_deadline_ms(1_700_000_031_000)
                    .with_content_deadline_ms(1_700_000_121_000)
            ),
            active.clone().with_step(
                RunStatusStep::new(Phase::RunningTools).with_last_event_at_ms(1_700_000_002_000)
            ),
            RunStatus::context_ceiling_wait(1_700_000_180_000)
                .with_step(RunStatusStep::new(Phase::Waiting)),
            active.with_step(
                RunStatusStep::new(Phase::Unknown("future_phase".into()))
                    .with_idle_deadline_ms(1_700_000_032_000)
                    .with_content_deadline_ms(1_700_000_122_000)
            ),
        ]
    );
    let expected = bytes.replace(",\n      \"attempt\": 2", "");
    assert_eq!(
        serde_json::to_string_pretty(&statuses).unwrap() + "\n",
        expected
    );
}
