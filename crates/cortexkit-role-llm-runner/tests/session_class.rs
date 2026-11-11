use cortexkit_role_llm_runner::send::SendRequest;
use serde_json::json;

#[test]
fn session_class_decode_refuses_malformed_labels_by_name() {
    for label in [
        json!(null),
        json!(17),
        json!(true),
        json!([]),
        json!({}),
        json!(""),
        json!("Upper"),
        json!("has space"),
        json!("has.dot"),
        json!("é"),
        json!("a".repeat(33)),
    ] {
        let result = serde_json::from_value::<SendRequest>(json!({
            "prompt": "hello", "send_id": "send-1", "session_class": label
        }));
        let error = result.expect_err("malformed session_class must be refused");
        assert!(error.to_string().contains("session_class"), "{error}");
    }
}

#[test]
fn session_class_round_trips_and_leaves_runner_parameters() {
    for label in ["a".to_owned(), "0123456789_-".to_owned(), "a".repeat(32)] {
        let wire =
            json!({"prompt":"hello", "send_id":"send-1", "session_class":label, "custom":true});
        let request: SendRequest = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(request.session_class.as_deref(), Some(label.as_str()));
        assert!(!request.runner_params.contains_key("session_class"));
        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(request.check_session_class(true, None), Ok(()));
        assert_eq!(
            SendRequest::new("hello", "send-1")
                .with_session_class(&label)
                .session_class,
            Some(label)
        );
    }
    let request = SendRequest::new("hello", "send-1");
    assert_eq!(request.session_class, None);
    assert!(serde_json::to_value(request)
        .unwrap()
        .get("session_class")
        .is_none());
}

#[test]
fn session_class_freezes_only_an_existing_label() {
    let request = SendRequest::new("hello", "send-1").with_session_class("archive");
    assert_eq!(request.check_session_class(true, None), Ok(()));
    assert_eq!(request.check_session_class(true, Some("archive")), Ok(()));
    assert_eq!(
        request.check_session_class(true, Some("other")),
        Err("session_class")
    );
    assert_eq!(
        SendRequest::new("hello", "send-2").check_session_class(true, Some("archive")),
        Ok(())
    );
    assert_eq!(
        SendRequest::new("hello", "send-3")
            .with_session_class("INVALID")
            .check_session_class(true, None),
        Err("session_class")
    );
}

#[test]
fn session_class_non_declaring_runner_accepts_different_valid_labels() {
    for (id, label) in [("send-1", "archive"), ("send-2", "other")] {
        let request: SendRequest = serde_json::from_value(json!({
            "prompt":"hello", "send_id":id, "session_class":label
        }))
        .unwrap();
        assert_eq!(request.check_session_class(false, None), Ok(()));
        assert_eq!(request.check_session_class(false, Some("previous")), Ok(()));
    }
    assert_eq!(
        SendRequest::new("hello", "send-3")
            .with_session_class("INVALID")
            .check_session_class(false, None),
        Err("session_class")
    );
}

#[test]
fn session_class_describe_requires_backup_classes_and_send() {
    use cortexkit_role_llm_runner::{
        capabilities,
        describe::{check_describe, DescribeProblem, Major, RoleDescribe, SessionClass},
        ops, PROVIDES, REQUIRED_OPS,
    };
    let mut describe = RoleDescribe::new(
        vec![Major::new(
            PROVIDES,
            REQUIRED_OPS.iter().map(|op| (*op).into()).collect(),
            "alpha",
        )],
        "test",
    )
    .with_session_capabilities_from("baseline")
    .with_capabilities(vec![capabilities::SESSION_CLASS.into()]);
    assert!(check_describe(&serde_json::to_value(&describe).unwrap())
        .unwrap_err()
        .contains(&DescribeProblem::Undecodable(
            "session_class_missing".into()
        )));
    describe = describe.with_session_class(SessionClass::new(vec!["archive".into()]));
    assert!(check_describe(&serde_json::to_value(&describe).unwrap()).unwrap_err().iter().any(|problem| matches!(problem, DescribeProblem::GroupIncomplete {capability, op} if *capability == capabilities::SESSION_CLASS && *op == ops::SESSION_SEND)));
    describe.majors[0].ops.push(ops::SESSION_SEND.into());
    let mut wire = serde_json::to_value(&describe).unwrap();
    assert_eq!(wire["session_class"], json!({"backup_classes":["archive"]}));
    assert_eq!(check_describe(&wire).unwrap(), describe);
    for settings in [
        json!({}),
        json!({"backup_classes":null}),
        json!({"backup_classes":[1]}),
    ] {
        wire["session_class"] = settings;
        assert!(check_describe(&wire).is_err());
    }
    wire["session_class"] = json!({"backup_classes":[], "future":true});
    assert!(check_describe(&wire).is_ok());
}
