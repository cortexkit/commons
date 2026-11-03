use cortexkit_role_step_transform_provider::{
    answer::{apply_ops, apply_to_parts, check_answer, AnswerProblem, HookAnswer, Operation},
    hook::{HookCall, Subject},
    subscription::{Declaration, Hook, OnUnavailable, Op, Phase, StaleDifference, Subscription},
};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};

fn round_trip<T: Serialize + DeserializeOwned>(wire: Value) -> T {
    let value: T = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(&value).unwrap(), wire);
    value
}

#[test]
fn unknown_string_and_tagged_values_round_trip_losslessly() {
    round_trip::<Hook>(json!("future_hook"));
    round_trip::<Phase>(json!("future_phase"));
    round_trip::<Op>(json!("future_op"));
    round_trip::<OnUnavailable>(json!("future_policy"));
    round_trip::<HookCall>(
        json!({"hook":"future_hook", "params":{}, "payload":[1,{"opaque":true}]}),
    );
    round_trip::<HookAnswer>(json!({"answer":"future_answer", "payload":[1,2]}));
    round_trip::<Operation>(json!({"op":"future_op", "block":0, "opaque":true}));
    round_trip::<StaleDifference>(json!({"kind":"future_difference", "opaque":true}));
    round_trip::<StaleDifference>(
        json!({"kind":"subscription_loosened", "provider":"example", "hook":"pre_user", "phase":null, "field":"future_field"}),
    );
}

#[test]
fn malformed_known_variants_and_missing_discriminators_still_fail() {
    for wire in [
        json!({}),
        json!({"hook":1}),
        json!({"hook":"pre_user"}),
        json!({"hook":"pre_tool", "phase":"mutate"}),
    ] {
        assert!(serde_json::from_value::<HookCall>(wire).is_err());
    }
    for wire in [
        json!({}),
        json!({"answer":false}),
        json!({"answer":"ops"}),
        json!({"answer":"ask", "ask":{}}),
    ] {
        assert!(serde_json::from_value::<HookAnswer>(wire).is_err());
    }
    for wire in [
        json!({}),
        json!({"op":null}),
        json!({"op":"append", "block":0}),
        json!({"op":"replace", "block":0, "value":12}),
    ] {
        assert!(serde_json::from_value::<Operation>(wire).is_err());
    }
}

#[test]
fn unknown_values_never_authorize_execution_or_fail_open() {
    for field in ["hook", "phase", "ops", "on_unavailable"] {
        let mut wire = json!({"hook":"pre_tool", "phase":"mutate", "ops":[], "on_unavailable":"refuse", "budget_ms":100});
        wire[field] = if field == "ops" {
            json!(["future_op"])
        } else {
            json!("future_value")
        };
        let subscription: Subscription = serde_json::from_value(wire).unwrap();
        assert!(subscription.check().is_err(), "{field}");
    }
    let declaration: Declaration = serde_json::from_value(
        json!({"subscriptions":[{"hook":"pre_user", "ops":["future_op"], "budget_ms":100}]}),
    )
    .unwrap();
    assert!(!declaration.check().is_empty());
    assert!(!Hook::Unknown("future_hook".into()).takes_ops());
    assert!(!OnUnavailable::Unknown("future_policy".into()).within(OnUnavailable::Pass));
    assert!(!OnUnavailable::Refuse.within(OnUnavailable::Unknown("future_policy".into())));
    let subject: Subject = serde_json::from_value(json!({"hook":"pre_tool", "step_id":"step", "phase":"approve", "tool":"tool", "tool_call_id":"call", "input":{}})).unwrap();
    for policy in ["on_expiry", "late_execution"] {
        let mut answer = json!({"answer":"ask", "ask":{"prompt":"run?", "expires_at_ms":100, "on_expiry":"deny", "material_damage":false, "late_execution":"notify_only"}});
        answer["ask"][policy] = json!("future_policy");
        let answer = round_trip::<HookAnswer>(answer);
        assert_eq!(
            check_answer(&subject, &[], None, &answer),
            Err(AnswerProblem::AnswerNotAllowed)
        );
    }
    let subject: Subject =
        serde_json::from_value(json!({"hook":"future_hook", "opaque":true})).unwrap();
    assert!(check_answer(&subject, &[], None, &HookAnswer::Pass).is_err());
}

#[test]
fn unknown_operation_rejects_the_entire_batch_before_mutation() {
    let ops = vec![
        Operation::Append {
            block: 0,
            text: "changed".into(),
            note: None,
        },
        round_trip::<Operation>(json!({"op":"future_op", "block":0})),
    ];
    let subject = Subject::PreUser {
        blocks: vec!["original".into()],
        mark: None,
        delivery: None,
        at_ms: None,
    };
    assert_eq!(
        check_answer(
            &subject,
            &[Op::Append, Op::Unknown("future_op".into())],
            None,
            &HookAnswer::Ops { ops: ops.clone() }
        ),
        Err(AnswerProblem::AnswerNotAllowed)
    );
    assert_eq!(
        apply_ops(&["original".into()], &ops),
        Err(AnswerProblem::AnswerNotAllowed)
    );
    let mut parts = vec!["original".to_owned()];
    assert_eq!(
        apply_to_parts(&mut parts, |part| Some(part), &ops),
        Err(AnswerProblem::AnswerNotAllowed)
    );
    assert_eq!(parts, ["original"]);
}
