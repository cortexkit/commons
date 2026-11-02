use cortexkit_role_tool_provider::views::{ViewsReply, ViewsRequest};
use serde_json::{json, Value};

#[test]
fn view_tool_definition_survives_reply_decode() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../test-vectors/tool-provider-v1/view-tools.json"
    ))
    .unwrap();
    let case = &fixture["cases"][0];
    let pin = case["pin"].as_str().unwrap();
    let wire = json!({"views": {pin: case["view"]}});
    let reply: ViewsReply = serde_json::from_value(wire.clone()).unwrap();
    reply
        .validate_for(&ViewsRequest::new(vec![pin.into()]))
        .unwrap();
    assert_eq!(serde_json::to_value(reply).unwrap(), wire);
}
