use cortexkit_role_agent_data::describe::{DescribeReply, Store};
use serde_json::json;

const REQUIRED_REPLY: &str = concat!(
    "{\"ref_kinds\":[\"display.session\"],",
    "\"stores\":[{\"name\":\"cards\",\"sync_class\":\"agent\"}],",
    "\"external\":false}"
);

fn reply() -> DescribeReply {
    DescribeReply::new(
        vec!["display.session".into()],
        vec![Store::new("cards", "agent")],
        false,
    )
}

#[test]
fn describe_absent_optional_round_trips_with_original_bytes() {
    let decoded: DescribeReply = serde_json::from_str(REQUIRED_REPLY).unwrap();
    assert!(!decoded.optional);
    assert_eq!(decoded, reply());
    assert_eq!(serde_json::to_string(&decoded).unwrap(), REQUIRED_REPLY);
    assert_eq!(serde_json::to_string(&reply()).unwrap(), REQUIRED_REPLY);
}

#[test]
fn describe_optional_true_round_trips() {
    let wire = json!({
        "ref_kinds": ["display.session"],
        "stores": [{"name": "cards", "sync_class": "agent"}],
        "external": false,
        "optional": true
    });
    let decoded: DescribeReply = serde_json::from_value(wire.clone()).unwrap();
    assert!(decoded.optional);
    assert_eq!(decoded, reply().with_optional(true));
    assert_eq!(serde_json::to_value(&decoded).unwrap(), wire);
}

#[test]
fn describe_optional_false_is_required_and_not_serialized() {
    let mut wire: serde_json::Value = serde_json::from_str(REQUIRED_REPLY).unwrap();
    wire["optional"] = json!(false);
    let decoded: DescribeReply = serde_json::from_value(wire).unwrap();
    assert!(!decoded.optional);
    assert_eq!(serde_json::to_string(&decoded).unwrap(), REQUIRED_REPLY);
    assert_eq!(
        serde_json::to_string(&reply().with_optional(true).with_optional(false)).unwrap(),
        REQUIRED_REPLY
    );
}

#[test]
fn describe_optional_is_boolean_and_reply_remains_strict() {
    for invalid in [json!(null), json!("true"), json!(1), json!({})] {
        let mut wire: serde_json::Value = serde_json::from_str(REQUIRED_REPLY).unwrap();
        wire["optional"] = invalid;
        assert!(serde_json::from_value::<DescribeReply>(wire).is_err());
    }
    let mut wire: serde_json::Value = serde_json::from_str(REQUIRED_REPLY).unwrap();
    wire["future"] = json!(true);
    assert!(serde_json::from_value::<DescribeReply>(wire)
        .unwrap_err()
        .to_string()
        .contains("unknown field `future`"));
}
