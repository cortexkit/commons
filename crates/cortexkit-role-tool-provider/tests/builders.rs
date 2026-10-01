use cortexkit_role_tool_provider::{
    catalog::{CatalogAnswer, CatalogTool, SystemTextAnswer},
    late_results::{Cursor, LateEntry, LateResultsRequest},
    scope::ScopeIdentity,
    withdraw::WithdrawArguments,
};
use serde_json::{json, Map};

#[test]
fn catalog_builders_preserve_wire_members() {
    let tool = CatalogTool::new("tool", "schema", 2, json!({"type": "object"}));
    assert_eq!(
        serde_json::to_value(&tool).unwrap(),
        json!({
            "name": "tool", "schema_digest": "schema", "semantics": 2,
            "input_schema": {"type": "object"}, "capabilities": []
        })
    );
    let text = SystemTextAnswer::new("item", "preflight");
    assert_eq!(
        serde_json::to_value(&text).unwrap(),
        json!({
            "item_digest": "item", "preflight_digest": "preflight"
        })
    );
    assert_eq!(
        serde_json::to_value(CatalogAnswer::new("gen", "catalog")).unwrap(),
        json!({
            "generation": "gen", "catalog_digest": "catalog", "tools": []
        })
    );
    let answer = CatalogAnswer::new("gen", "catalog")
        .with_composition_digest("composition")
        .with_tools(vec![tool
            .with_result_ops(vec!["append".into()])
            .with_capabilities(vec!["code.outline/v1".into()])
            .with_description("description")])
        .with_system_text(
            text.with_text("text")
                .with_composition_digest("composition"),
        )
        .with_capabilities(Map::from_iter([("late_results".into(), json!(true))]));
    let expected = json!({
        "generation": "gen", "catalog_digest": "catalog", "composition_digest": "composition",
        "tools": [{"name": "tool", "schema_digest": "schema", "semantics": 2,
            "input_schema": {"type": "object"}, "result_ops": ["append"],
            "capabilities": ["code.outline/v1"], "description": "description"}],
        "system_text": {"item_digest": "item", "preflight_digest": "preflight",
            "text": "text", "composition_digest": "composition"},
        "capabilities": {"late_results": true}
    });
    assert_eq!(serde_json::to_value(&answer).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<CatalogAnswer>(expected).unwrap(),
        answer
    );
}

#[test]
fn late_results_builders_preserve_wire_members() {
    assert_eq!(
        serde_json::to_value(LateResultsRequest::new()).unwrap(),
        json!({"since": null})
    );
    let request = LateResultsRequest::new()
        .with_since(Cursor {
            provider_incarnation: "incarnation".into(),
            seq: 3,
        })
        .with_limit(10);
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({
            "since": {"provider_incarnation": "incarnation", "seq": 3}, "limit": 10
        })
    );
    let entry = LateEntry::new(
        "result",
        "owner",
        "scope",
        4,
        "custodian",
        "key",
        "event",
        5,
    );
    assert_eq!(
        serde_json::to_value(&entry).unwrap(),
        json!({
            "kind": "result", "owner": "owner", "ref": "scope", "scope_epoch": 4,
            "custodian": "custodian", "call_key": "key", "event_id": "event", "settled_at": 5,
            "reduced": false
        })
    );
    let entry = entry
        .with_invocation_id("invocation")
        .with_reduced(true)
        .with_result(json!({"ok": true}))
        .with_outcome("ok")
        .with_reason("reason")
        .with_extra(Map::from_iter([("extension".into(), json!(7))]));
    let expected = json!({
        "kind": "result", "owner": "owner", "ref": "scope", "scope_epoch": 4,
        "custodian": "custodian", "call_key": "key", "event_id": "event", "settled_at": 5,
        "invocation_id": "invocation", "reduced": true, "result": {"ok": true},
        "outcome": "ok", "reason": "reason", "extension": 7
    });
    assert_eq!(serde_json::to_value(&entry).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<LateEntry>(expected).unwrap(),
        entry
    );
}

#[test]
fn withdraw_builder_preserves_wire_members() {
    let arguments = WithdrawArguments::new("key");
    assert_eq!(
        serde_json::to_value(&arguments).unwrap(),
        json!({"call_key": "key"})
    );
    let arguments = arguments.with_carrier("carrier").with_scope(ScopeIdentity {
        owner: "owner".into(),
        scope_ref: "scope".into(),
        scope_epoch: 4,
    });
    let expected = json!({"call_key": "key", "carrier": "carrier",
        "scope": {"owner": "owner", "ref": "scope", "scope_epoch": 4}});
    assert_eq!(serde_json::to_value(&arguments).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<WithdrawArguments>(expected).unwrap(),
        arguments
    );
}
