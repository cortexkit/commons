use cortexkit_role_compaction::{
    digest::content_digest,
    part::{body_digest, Delivered, Part, PartBody, RideForm, Seal, ToolPart, ToolStatus},
    read::{entry_digest, Entry, Role},
    tokens::TokenCount,
};
use serde_json::json;

#[test]
fn delivered_rides_forms_are_optional_and_unknown_values_are_skipped() {
    for (forms, expected) in [
        (None, vec![RideForm::ToolResult]),
        (
            Some(json!(["future", "turn_system", "tool_result"])),
            vec![RideForm::TurnSystem, RideForm::ToolResult],
        ),
        (Some(json!(["future"])), vec![RideForm::ToolResult]),
        (Some(json!([])), vec![RideForm::ToolResult]),
        (
            Some(json!(["turn_system_cleared"])),
            vec![RideForm::TurnSystemCleared],
        ),
        (
            Some(json!(["future", "turn_system_cleared", "tool_result"])),
            vec![RideForm::TurnSystemCleared, RideForm::ToolResult],
        ),
    ] {
        let mut wire = json!({"sender":"example-notifier", "text":"note", "text_bytes":4});
        if let Some(forms) = forms {
            wire["forms"] = forms;
        }
        let delivered: Delivered = serde_json::from_value(wire).unwrap();
        assert_eq!(delivered.forms(), expected);
        assert_eq!(delivered.send_id, None);
        assert_eq!(delivered.tokens, None);
    }
    assert_eq!(
        serde_json::to_value(RideForm::TurnSystemCleared).unwrap(),
        json!("turn_system_cleared")
    );
    let legacy = serde_json::to_value(Delivered::new("example-notifier", "note")).unwrap();
    assert!(legacy.get("forms").is_none());
    assert!(legacy.get("send_id").is_none());
    assert!(legacy.get("tokens").is_none());
}

#[test]
fn delivered_rides_tokens_are_metadata_not_content() {
    let delivered = Delivered::new("example-notifier", "note")
        .with_send_id("send-a")
        .with_forms(vec![RideForm::TurnSystem, RideForm::ToolResult])
        .with_tokens(TokenCount::new(2));
    let part = Part::new(
        "p",
        PartBody::Tool(
            ToolPart::new(
                "call",
                "tool",
                json!({"tokens":{"count":7},"delivered":[{"tokens":9}]}),
                ToolStatus::Completed,
            )
            .with_output("output")
            .with_delivered(vec![delivered]),
        ),
    );
    let entry = Entry::new(1, "m", Role::Assistant, 1, vec![part.clone()]);
    let mut wire = serde_json::to_value(&part).unwrap();
    wire["delivered"][0]
        .as_object_mut()
        .unwrap()
        .remove("tokens");
    let expected_part = {
        let mut body = wire.clone();
        for field in ["id", "tag", "digest", "tokens"] {
            body.as_object_mut().unwrap().remove(field);
        }
        content_digest(&body)
    };
    assert_eq!(part.digest, expected_part);
    assert_eq!(body_digest(Seal::None, &part.body), expected_part);
    assert_eq!(
        entry.digest,
        content_digest(&json!({"role":"assistant","parts":[wire]}))
    );
    let mut changed = entry.clone();
    let PartBody::Tool(tool) = &mut changed.parts[0].body else {
        panic!("tool");
    };
    tool.delivered[0].tokens = Some(TokenCount::new(100));
    assert_eq!(
        entry_digest(changed.role.clone(), None, &changed.parts),
        entry.digest
    );
    let PartBody::Tool(tool) = &mut changed.parts[0].body else {
        panic!("tool");
    };
    tool.input["tokens"]["count"] = json!(8);
    assert_ne!(
        entry_digest(changed.role, None, &changed.parts),
        entry.digest
    );
    let PartBody::Tool(compact) = part.compact().body else {
        panic!("tool");
    };
    assert_eq!(compact.delivered[0].tokens, Some(TokenCount::new(2)));
    assert_eq!(compact.delivered[0].text, None);
    assert_eq!(compact.delivered[0].send_id.as_deref(), Some("send-a"));
}
