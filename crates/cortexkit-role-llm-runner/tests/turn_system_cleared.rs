use cortexkit_role_llm_runner::{
    capabilities,
    delivery_order::{choose, Boundary, Mode},
    describe::{check_describe, Major, RoleDescribe},
    ops,
    send::SendRequest,
};
use serde_json::json;

#[test]
fn turn_system_cleared_decodes_with_existing_describe_narrowing() {
    let describe = RoleDescribe::new(
        vec![Major::new(
            "llm-runner/v1",
            vec![
                ops::ROLE_DESCRIBE.into(),
                ops::SESSION_BASELINE.into(),
                ops::SESSION_SEND.into(),
            ],
            "alpha",
        )],
        "test",
    )
    .with_capabilities(vec![capabilities::DELIVERY_ORDER.into()])
    .with_session_capabilities_from("baseline")
    .with_delivery_order_modes(vec!["turn_system_cleared".into()]);
    check_describe(&serde_json::to_value(&describe).unwrap()).unwrap();
    assert!(Mode::TurnSystemCleared.is_declared_by(&describe));
    assert!(!Mode::TurnSystem.is_declared_by(&describe));
    let request: SendRequest = serde_json::from_value(json!({
        "session":"s", "send_id":"n", "prompt":"free text",
        "delivery":["turn_system_cleared", "tool_result"]
    }))
    .unwrap();
    let order = request.delivery_order().unwrap().unwrap();
    assert_eq!(order.modes, [Mode::TurnSystemCleared, Mode::ToolResult]);
    assert_eq!(
        serde_json::to_value(Mode::TurnSystemCleared).unwrap(),
        "turn_system_cleared"
    );
    assert!(!Mode::TurnSystemCleared.is_user_authority());
    let narrowed = describe.with_delivery_order_modes(vec!["tool_result".into()]);
    assert!(!Mode::TurnSystemCleared.is_declared_by(&narrowed));
}

#[test]
fn turn_system_cleared_requires_confirmed_template_route_and_tool_boundary() {
    let order = [Mode::TurnSystemCleared, Mode::ToolResult, Mode::Steer];
    let tool = Boundary::default()
        .with_tool_result()
        .with_run_in_progress();
    assert_eq!(choose(&order, &tool), Some(Mode::ToolResult));
    assert_eq!(
        choose(&order, &tool.with_turn_system()),
        Some(Mode::ToolResult)
    );
    assert_eq!(
        choose(&order, &tool.with_turn_system_cleared()),
        Some(Mode::TurnSystemCleared)
    );
    let no_tool = Boundary::default()
        .with_turn_system_cleared()
        .with_run_in_progress();
    assert_eq!(choose(&order, &no_tool), Some(Mode::Steer));
    assert_eq!(choose(&[Mode::TurnSystemCleared], &tool), None);
}
