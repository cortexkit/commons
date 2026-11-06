use cortexkit_role_llm_runner::{capabilities, describe::RoleDescribe, read::HeadMeta};
use serde_json::json;

#[test]
fn history_intake_is_additive_and_unknown_states_block_seeding() {
    let head = HeadMeta::empty(7, "record").with_last_ordinal(2);
    let bytes = serde_json::to_value(&head).unwrap();
    assert!(bytes.get("history_intake").is_none());
    let mut describe: RoleDescribe = serde_json::from_value(json!({
        "majors":[], "implementation_version":"test", "capabilities":[]
    }))
    .unwrap();
    assert!(!head.can_seed_new_from_here(&describe));
    describe
        .capabilities
        .push(capabilities::HISTORY_INTAKE.into());
    assert!(head.can_seed_new_from_here(&describe));
    let running = head.with_history_intake_running();
    assert!(running.history_intake_running());
    assert!(!running.can_seed_new_from_here(&describe));
    let mut future = serde_json::to_value(running).unwrap();
    future["history_intake"] = json!("future_phase");
    let decoded: HeadMeta = serde_json::from_value(future).unwrap();
    assert_eq!(decoded.history_intake.as_deref(), Some("future_phase"));
    assert!(decoded.history_intake_running());
    assert!(!decoded.can_seed_new_from_here(&describe));
    assert_eq!(
        capabilities::group_ops(capabilities::HISTORY_INTAKE),
        Some(&["session.head"][..])
    );
    let mut ops = cortexkit_role_llm_runner::REQUIRED_OPS.to_vec();
    ops.push("session.head");
    let raw = json!({
        "majors":[{"version":cortexkit_role_llm_runner::PROVIDES,"ops":ops,"stability":"stable"}],
        "implementation_version":"test","capabilities":[capabilities::HISTORY_INTAKE],
        "session_capabilities_from":"baseline"
    });
    assert!(cortexkit_role_llm_runner::describe::check_describe(&raw).is_ok());
}
