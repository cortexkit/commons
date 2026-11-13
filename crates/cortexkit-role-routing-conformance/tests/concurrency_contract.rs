use cortexkit_role_routing::{
    declarations::{
        parse_consumer_declarations, parse_consumer_declarations_serving, ConsumerDeclarations,
        DeclarationError,
    },
    errors::{NoRouteDetail, NoRouteReason, Refusal},
    refusals::{retry_class, RetryClass},
};
use serde_json::{json, Value};

const PRIORITY_GROUP: &str = "consumer_priority";

#[test]
fn concurrency_full_decodes_as_a_known_timed_wait() {
    let wire = json!({
        "code": "no_route", "message": "capacity occupied",
        "detail": {"reason": "concurrency_full", "reset_at_ms": 1_800_000_030_000_u64}
    });
    let refusal: Refusal = serde_json::from_value(wire.clone()).unwrap();
    let detail = refusal.detail_as::<NoRouteDetail>().unwrap();
    assert!(!matches!(detail.reason, NoRouteReason::Other(_)));
    assert_eq!(detail.reset_at_ms, Some(1_800_000_030_000));
    assert_eq!(refusal.wait_until_ms(), detail.reset_at_ms);
    assert_eq!(
        retry_class("no_route", &wire["detail"]),
        RetryClass::RetryAt(1_800_000_030_000)
    );
    assert_eq!(serde_json::to_value(refusal).unwrap(), wire);
}

#[test]
fn concurrency_priority_round_trips_all_classes() {
    for priority in ["interactive", "blocking", "worker", "background"] {
        let wire = json!({"consumers":[{"name":"example.task","priority":priority}]});
        let parsed = parse_consumer_declarations_serving(&wire, &[PRIORITY_GROUP]).unwrap();
        assert!(!parsed.consumers[0]
            .defaults
            .unknown
            .contains_key("priority"));
        assert_eq!(serde_json::to_value(&parsed).unwrap(), wire);
        parsed.validate_serving(&[PRIORITY_GROUP]).unwrap();
    }
}

#[test]
fn concurrency_priority_is_reader_first_even_when_null() {
    for priority in [json!("worker"), Value::Null] {
        let wire = json!({"consumers":[{"name":"example.task","priority":priority}]});
        let expected = DeclarationError::UnknownMember {
            field: "consumers[0].priority".into(),
        };
        assert_eq!(parse_consumer_declarations(&wire), Err(expected.clone()));
        assert_eq!(
            parse_consumer_declarations_serving(&wire, &["consumer_declarations"]),
            Err(expected.clone())
        );
        let typed: ConsumerDeclarations = serde_json::from_value(wire).unwrap();
        assert_eq!(typed.validate(), Err(expected.clone()));
        assert_eq!(
            typed.validate_serving(&["consumer_declarations"]),
            Err(expected)
        );
        typed.validate_serving(&[PRIORITY_GROUP]).unwrap();
    }
    let absent = json!({"consumers":[{"name":"example.task"}]});
    let typed = parse_consumer_declarations(&absent).unwrap();
    assert_eq!(serde_json::to_value(typed).unwrap(), absent);
}

#[test]
fn concurrency_priority_refuses_invalid_values_by_name() {
    for priority in [
        json!("urgent"),
        json!("Worker"),
        json!(""),
        json!(1),
        json!([]),
    ] {
        let wire = json!({"consumers":[{"name":"example.task","priority":priority}]});
        assert_eq!(
            parse_consumer_declarations_serving(&wire, &[PRIORITY_GROUP]),
            Err(DeclarationError::InvalidField {
                field: "consumers[0].priority".into()
            })
        );
    }
}
