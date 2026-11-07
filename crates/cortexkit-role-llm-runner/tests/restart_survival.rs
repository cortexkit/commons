use cortexkit_role_llm_runner::{
    capabilities,
    describe::{check_describe, DescribeProblem},
    read::{HeadMeta, HostState},
    run::{interrupt_reasons, RunReason, RunResult},
    vectors,
};
use serde_json::{json, Value};

fn sample() -> Value {
    serde_json::from_str(vectors::RESTART_SURVIVAL_JSON).unwrap()
}

#[test]
fn restart_survival_dependencies_and_ops_are_checked() {
    let original = sample()["describe"].clone();
    assert!(check_describe(&original)
        .unwrap()
        .declares(capabilities::RESTART_SURVIVAL));
    for (dependency, problem) in [
        (
            "interrupt_reason",
            "restart_survival_without_interrupt_reason",
        ),
        ("live_events", "restart_survival_without_live_events"),
    ] {
        let mut raw = original.clone();
        raw["capabilities"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v != dependency);
        assert!(check_describe(&raw)
            .unwrap_err()
            .contains(&DescribeProblem::Undecodable(problem.into())));
    }
    for op in [
        "session.head",
        "session.read",
        "session.subscribe",
        "run.result",
        "run.status",
    ] {
        let mut raw = original.clone();
        raw["majors"][0]["ops"]
            .as_array_mut()
            .unwrap()
            .retain(|v| v != op);
        assert!(check_describe(&raw).is_err(), "missing {op}");
    }
}

#[test]
fn restart_survival_head_and_reason_round_trip_openly() {
    let samples = sample();
    let head: HeadMeta = serde_json::from_value(samples["unknown_head"].clone()).unwrap();
    assert_eq!(head.host, Some(HostState::Unknown));
    assert_eq!(
        serde_json::to_value(&head).unwrap(),
        samples["unknown_head"]
    );
    let old = HeadMeta::empty(7, "record-1");
    assert!(serde_json::to_value(old).unwrap().get("host").is_none());
    let next: HostState = serde_json::from_value(json!("future-state")).unwrap();
    assert_eq!(next, HostState::Other("future-state".into()));
    assert_eq!(serde_json::to_value(next).unwrap(), "future-state");
    let stopped: RunResult = serde_json::from_value(samples["stopped"].clone()).unwrap();
    assert_eq!(
        stopped.reason.as_deref(),
        Some(interrupt_reasons::HOST_UNKNOWN)
    );
    // Interrupted reasons are open strings, including for older status readers.
    assert_eq!(
        RunReason::from("host_unknown".to_owned()),
        RunReason::Unknown("host_unknown".into())
    );
    assert_eq!(serde_json::to_value(stopped).unwrap(), samples["stopped"]);
}

#[test]
fn restart_survival_samples_decode_and_round_trip() {
    let samples = sample();
    let survived: RunResult = serde_json::from_value(samples["survived"].clone()).unwrap();
    assert_eq!(survived.run_id, "run-1");
    assert_eq!(survived.state, "completed");
    assert_eq!(serde_json::to_value(survived).unwrap(), samples["survived"]);
    let unknown: cortexkit_role_llm_runner::send::SendReply =
        serde_json::from_value(samples["unknown_send"].clone()).unwrap();
    assert_eq!(unknown.delivered.as_ref().unwrap().r#as, "unknown");
    assert_eq!(
        serde_json::to_value(unknown).unwrap(),
        samples["unknown_send"]
    );
    let refusal = &samples["recovering_refusal"];
    assert_eq!(refusal["code"], "transient");
    assert_eq!(refusal["detail"]["reason"], "recovering");
    assert!(cortexkit_role_llm_runner::errors::is_retryable(
        refusal["code"].as_str().unwrap()
    ));
}
