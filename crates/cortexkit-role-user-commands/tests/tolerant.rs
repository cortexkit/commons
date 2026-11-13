use cortexkit_role_user_commands::{read_list_reply, ListReply};
#[cfg(feature = "digest")]
use cortexkit_role_user_commands::{DescribeDecodeError, DescribeReply};
use serde_json::{json, Value};

fn command(kind: &str) -> Value {
    json!({"id":"edit", "title":"Edit", "summary":"Edit data.", "subject":kind, "args":[], "effects":["reversible"], "run":{}})
}

#[test]
fn tolerant_list_keeps_readable_entries_and_reports_failures() {
    let body = json!({"commands":[
        {"module":"good", "command":command("project")},
        {"module":"future", "command":command("future_kind")},
        {"module":"broken", "command":{"id":"missing_fields"}},
        {"command":command("workspace")}
    ]});
    assert!(serde_json::from_value::<ListReply>(body.clone()).is_err());
    let result = read_list_reply(body).unwrap();
    assert_eq!(result.commands.len(), 1);
    assert_eq!(result.commands[0].module, "good");
    assert_eq!(result.unreadable.len(), 3);
    assert_eq!(result.unreadable[0].index, 1);
    assert_eq!(result.unreadable[0].module.as_deref(), Some("future"));
    assert!(result.unreadable[0].error.contains("future_kind"));
    assert_eq!(result.unreadable[1].module.as_deref(), Some("broken"));
    assert_eq!(result.unreadable[2].module, None);
    assert!(read_list_reply(json!({"commands": false})).is_err());
}

#[cfg(feature = "digest")]
#[test]
fn tolerant_describe_keeps_readable_entries_and_reports_failures() {
    let mut unknown = command("workspace");
    unknown["id"] = json!("future");
    unknown["args"] = json!([{"key":"what", "kind":"future_arg", "label":"What"}]);
    let mut body = json!({"commands":[command("project"), unknown, {"id":"broken"}], "digest":"bd99e260aae241c6cf2bfd10f12ee53e4998b741a787ca7c9175163fa3c44c61"});
    assert!(serde_json::from_value::<DescribeReply>(body.clone()).is_err());
    let result = DescribeReply::decode_tolerant(&serde_json::to_vec(&body).unwrap()).unwrap();
    assert_eq!(result.commands.len(), 1);
    assert_eq!(result.commands[0].id, "edit");
    assert_eq!(result.unreadable.len(), 2);
    assert_eq!(result.unreadable[0].index, 1);
    assert_eq!(result.unreadable[0].id.as_deref(), Some("future"));
    assert!(result.unreadable[0].error.contains("future_arg"));
    assert_eq!(result.unreadable[1].id.as_deref(), Some("broken"));
    assert_eq!(result.digest, body["digest"]);
    body["commands"][0]["title"] = json!("tampered");
    assert!(matches!(
        DescribeReply::decode_tolerant(&serde_json::to_vec(&body).unwrap()),
        Err(DescribeDecodeError::DigestMismatch)
    ));
    assert!(DescribeReply::decode_tolerant(br#"{"commands":[], "digest":false}"#).is_err());
}
