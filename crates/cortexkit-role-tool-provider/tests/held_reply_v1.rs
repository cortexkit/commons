use cortexkit_role_tool_provider::{
    consumer_reads_held_reply_v1,
    result::{validate, ContentPart, HeldReply, ToolResult},
    CONTENT_V1_CAPABILITY, HELD_REPLY_V1,
};
use serde_json::{json, Value};

fn vectors() -> Value {
    serde_json::from_str(include_str!(
        "../../../test-vectors/tool-provider-v1/held-reply-v1.json"
    ))
    .unwrap()
}

fn check_vector(group: &str, name: &str) {
    let vectors = vectors();
    let case = vectors[group]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap();
    let decoded = serde_json::from_value::<ToolResult>(case["body"].clone());
    if group == "decode_invalid" {
        decoded.expect_err(name);
        return;
    }
    let result = decoded.unwrap();
    if group == "valid" {
        assert_eq!(validate(&result), Ok(()), "{name}");
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            case["body"],
            "{name}"
        );
    } else {
        let problem = validate(&result).expect_err(name);
        assert_eq!(problem.member, case["member"], "{name}");
        assert_eq!(problem.reason, case["reason"], "{name}");
    }
}

macro_rules! vector_tests {
    ($list:ident, $group:literal, $($name:ident),+ $(,)?) => {
        const $list: &[&str] = &[$(stringify!($name)),+];
        $(#[test] fn $name() { check_vector($group, stringify!($name)); })+
    };
}

vector_tests!(
    VALID,
    "valid",
    held_consent,
    held_login_handoff,
    held_login_capture,
    held_unknown_reason
);
vector_tests!(
    INVALID,
    "invalid",
    held_with_is_error_true,
    held_bad_reason,
    held_nonobject_detail,
    held_null_detail,
    held_empty_content
);
vector_tests!(
    DECODE_INVALID,
    "decode_invalid",
    held_missing_content,
    held_missing_reason,
    held_nonstring_reason
);

#[test]
fn every_held_vector_has_a_named_test() {
    let vectors = vectors();
    for (group, names) in [
        ("valid", VALID),
        ("invalid", INVALID),
        ("decode_invalid", DECODE_INVALID),
    ] {
        let actual: Vec<_> = vectors[group]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| case["name"].as_str().unwrap())
            .collect();
        assert_eq!(actual, names);
    }
}

#[test]
fn held_route_gating_decision_table() {
    for case in vectors()["gating"].as_array().unwrap() {
        let caps: Option<Vec<String>> =
            serde_json::from_value(case["consumer_capabilities"].clone()).unwrap();
        assert_eq!(
            consumer_reads_held_reply_v1(caps.as_deref()),
            case["held"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
    assert_eq!(HELD_REPLY_V1, "held-reply/v1");
    assert_eq!(CONTENT_V1_CAPABILITY, "content/v1");
}

#[test]
fn held_builders_and_accessor_keep_detail_opaque() {
    let detail = json!({"provider": {"nested": [true, 42, null]}});
    let result = ToolResult::new(vec![ContentPart::text("Wait for a person.")])
        .with_held(HeldReply::new("consent").with_detail(detail.as_object().unwrap().clone()));
    assert_eq!(result.held().unwrap().reason, "consent");
    assert_eq!(result.held().unwrap().detail.as_ref(), Some(&detail));
    assert_eq!(validate(&result), Ok(()));
    assert_eq!(
        serde_json::from_value::<ToolResult>(serde_json::to_value(&result).unwrap()).unwrap(),
        result
    );
    assert!(ToolResult::new(vec![ContentPart::text("Final.")])
        .held()
        .is_none());
}

#[test]
fn held_reason_uses_error_code_grammar() {
    for reason in ["consent", "provider_step_v2", "http_404"] {
        assert_eq!(
            validate(
                &ToolResult::new(vec![ContentPart::text("Wait.")])
                    .with_held(HeldReply::new(reason))
            ),
            Ok(())
        );
    }
    for reason in [
        "",
        "_reason",
        "reason_",
        "two__words",
        "Bad",
        "bad-code",
        "bad code",
        "9reason",
        "échec",
    ] {
        let problem = validate(
            &ToolResult::new(vec![ContentPart::text("Wait.")]).with_held(HeldReply::new(reason)),
        )
        .expect_err(reason);
        assert_eq!(problem.member, "held.reason");
    }
}

#[test]
fn held_detail_rejects_all_nonobject_json_types() {
    for detail in [
        json!(null),
        json!(true),
        json!(42),
        json!("opaque"),
        json!([]),
    ] {
        let result: ToolResult = serde_json::from_value(json!({
            "content": [{"type": "text", "text": "Wait."}],
            "held": {"reason": "consent", "detail": detail}
        }))
        .unwrap();
        let problem = validate(&result).unwrap_err();
        assert_eq!(problem.member, "held.detail");
    }
}
