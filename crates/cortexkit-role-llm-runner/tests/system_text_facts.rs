use cortexkit_role_llm_runner::{describe::check_describe, REQUIRED_OPS};
use serde_json::{json, Value};

fn describe(host_facts: Value) -> Value {
    json!({"majors":[{"version":"llm-runner/v1","ops":REQUIRED_OPS}],"implementation_version":"test","session_capabilities_from":"admission","harnesses":{"host":{"host_facts":host_facts}}})
}

#[test]
fn system_text_host_facts_preserved() {
    let parsed = check_describe(&describe(json!(["files", "shell"]))).unwrap();
    assert_eq!(
        serde_json::to_value(parsed).unwrap()["harnesses"]["host"]["host_facts"],
        json!(["files", "shell"])
    );
}

#[test]
fn system_text_host_facts_refuses_unsorted() {
    assert!(check_describe(&describe(json!(["z", "a"]))).is_err());
}

#[test]
fn system_text_host_facts_refuses_duplicates() {
    assert!(check_describe(&describe(json!(["a", "a"]))).is_err());
}

#[test]
fn system_text_host_facts_refuses_invalid_tokens() {
    for token in [
        "".to_owned(),
        "A".into(),
        "a b".into(),
        "a/b".into(),
        "é".into(),
        "a".repeat(65),
    ] {
        assert!(
            check_describe(&describe(json!([token]))).is_err(),
            "{token}"
        );
    }
}

#[test]
fn system_text_host_facts_refuses_null_or_wrong_type() {
    for invalid in [Value::Null, json!([null]), json!([1]), json!("shell")] {
        assert!(
            check_describe(&describe(invalid.clone())).is_err(),
            "{invalid}"
        );
    }
}

#[test]
fn system_text_host_facts_absence_and_constructor() {
    use cortexkit_role_llm_runner::describe::HarnessDescribe;
    let mut wire = describe(json!([]));
    wire["harnesses"]["host"] = json!({});
    let parsed = check_describe(&wire).unwrap();
    assert!(parsed.harnesses["host"].host_facts().is_empty());
    wire.as_object_mut().unwrap().remove("harnesses");
    assert!(check_describe(&wire).unwrap().harnesses.is_empty());
    let names = vec!["z".into(), "a_.-09".into(), "a_.-09".into(), "a".repeat(64)];
    let declaration = HarnessDescribe::new(names).unwrap();
    assert_eq!(
        declaration.host_facts(),
        ["a_.-09".to_owned(), "a".repeat(64), "z".into()]
    );
    assert!(HarnessDescribe::new(vec!["invalid token".into()]).is_err());
}

#[test]
fn system_text_facts_forwarding_is_exact_and_opt_in() {
    use cortexkit_role_llm_runner::system_text_facts::{for_provider, SYSTEM_TEXT_FACTS_V1};
    let item = json!({"facts":{"model":{"provider":"vendor","id":"model"},"harness":{"id":"host","facts":[]},"tools":["a","é"],"idle_start":[]}});
    let declared = [SYSTEM_TEXT_FACTS_V1.into()];
    let received = for_provider(&item, &declared).unwrap().unwrap();
    assert!(std::ptr::eq(received, &item["facts"]));
    assert_eq!(serde_json::to_vec(received).unwrap(), r#"{"harness":{"facts":[],"id":"host"},"idle_start":[],"model":{"id":"model","provider":"vendor"},"tools":["a","é"]}"#.as_bytes());
    assert!(for_provider(&item, &[]).unwrap().is_none());
    assert!(for_provider(&item, &["system-text-facts/v2".into()])
        .unwrap()
        .is_none());
    assert!(for_provider(&json!({}), &declared).unwrap().is_none());
}

#[test]
fn system_text_facts_forwarding_refuses_malformed_before_fetch() {
    use cortexkit_role_llm_runner::system_text_facts::{for_provider, SYSTEM_TEXT_FACTS_V1};
    for facts in [
        Value::Null,
        json!({}),
        json!({"model":{"id":"model","provider":"vendor"},"harness":{"id":"host","facts":[]},"tools":["z","a"],"idle_start":[]}),
    ] {
        for provides in [vec![], vec![SYSTEM_TEXT_FACTS_V1.into()]] {
            assert_eq!(
                for_provider(&json!({"facts":facts}), &provides).unwrap_err(),
                "facts"
            );
        }
    }
}
