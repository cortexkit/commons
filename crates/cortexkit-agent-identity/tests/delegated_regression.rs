use cortexkit_agent_identity::AgentSessionForRefReply;

#[test]
fn delegated_role_survives_reencoding() {
    let row = serde_json::json!({
        "agent_id": "agent_0000000000000101", "ref_kind": "session", "ref": "abc",
        "harness": null, "session_id": null, "project_root": null,
        "incarnation": "0123456789abcdef", "role": "delegated"
    });
    let decoded: AgentSessionForRefReply = serde_json::from_value(row.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), row);
}
