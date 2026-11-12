use cortexkit_role_llm_runner::{
    app_readers::{admit, ReadAdmission},
    capabilities,
    describe::{check_describe, DescribeProblem, Major, RoleDescribe},
    ops,
    plan_view::{
        NewestStep, Pending, PlanView, PlanViewRequest, System, SystemItem, Tool, ToolTokens,
    },
    PROVIDES, REQUIRED_OPS,
};
use serde_json::{json, Value};

fn view() -> Value {
    json!({
        "generation":3,
        "system":{"text":"é\n猫|ok", "items":[
            {"provider":"first", "preset":"p", "start":0, "end":2},
            {"provider":"second", "start":3, "end":6},
            {"provider":"third", "start":7, "end":9}
        ]},
        "tools":[{"name":"tool", "description":"description", "input_schema":{"type":"object"}, "provider":"tools", "tokens":{"count":4,"exact":false}}],
        "newest_step":{"at_ms":120, "reply_mid":"reply-1"},
        "pending":{"generation":4, "notes":["pending change"]},
        "as_planned":true
    })
}

#[test]
fn plan_view_request_is_empty_and_route_bound() {
    assert_eq!(
        serde_json::to_value(PlanViewRequest::new()).unwrap(),
        json!({})
    );
    assert!(serde_json::from_value::<PlanViewRequest>(json!({})).is_ok());
    for value in [
        json!({"session":"other"}),
        json!({"from_ordinal":1}),
        json!(null),
    ] {
        assert!(serde_json::from_value::<PlanViewRequest>(value).is_err());
    }
}

#[test]
fn plan_view_round_trips_and_builders_preserve_exact_text() {
    let wire = view();
    let reply: PlanView = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(reply.applied_at_ms, None);
    assert_eq!(reply.system.text, "é\n猫|ok");
    assert_eq!(serde_json::to_value(&reply).unwrap(), wire);
    assert_eq!(reply.check(), Ok(()));
    let built = PlanView::new(
        3,
        System::new("é\n猫|ok").with_items(vec![
            SystemItem::new("first", 0, 2).with_preset("p"),
            SystemItem::new("second", 3, 6),
            SystemItem::new("third", 7, 9),
        ]),
        vec![
            Tool::new("tool", "description", json!({"type":"object"}), "tools")
                .with_tokens(ToolTokens::estimate(4)),
        ],
    )
    .with_newest_step(NewestStep::new(120).with_reply_mid("reply-1"))
    .with_pending(
        Pending::new(4)
            .with_notes(vec!["pending change".into()])
            .unwrap(),
    );
    assert_eq!(built, reply);
    let mut timed = built.with_applied_at_ms(100);
    assert_eq!(timed.applied_at_ms, Some(100));
    timed.tools[0].tokens = Some(ToolTokens::measured(4));
    assert_eq!(
        serde_json::to_value(&timed).unwrap()["tools"][0]["tokens"],
        json!({"count":4,"exact":true})
    );
}

#[test]
fn plan_view_omits_unrecorded_optional_facts() {
    let reply = PlanView::new(0, System::new(""), vec![]).with_pending(Pending::new(1));
    let wire = serde_json::to_value(&reply).unwrap();
    assert_eq!(
        wire,
        json!({"generation":0,"system":{"text":""},"tools":[],"pending":{"generation":1},"as_planned":true})
    );
    assert_eq!(serde_json::from_value::<PlanView>(wire).unwrap(), reply);
    let mut wire = view();
    wire["newest_step"] = json!({"at_ms":120});
    wire["pending"] = json!({"generation":4});
    wire["system"].as_object_mut().unwrap().remove("items");
    assert!(serde_json::from_value::<PlanView>(wire).is_ok());
}

#[test]
fn plan_view_pending_notes_wire_validation() {
    assert!(Pending::new(4).with_notes(vec![]).is_err());
    assert!(Pending::new(4).with_notes(vec![String::new()]).is_err());
    let mut built = PlanView::new(3, System::new(""), vec![]).with_pending(Pending::new(4));
    built.pending.as_mut().unwrap().notes = Some(vec![]);
    assert_eq!(built.check(), Err("pending.notes"));
    for notes in [
        json!(["owner note"]),
        json!(["owner note", "removed é", "line\nthree"]),
    ] {
        let mut wire = view();
        wire["pending"] = json!({"generation":4, "notes": notes});
        let bytes = serde_json::to_vec(&wire).unwrap();
        let reply: PlanView = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            serde_json::to_vec(&serde_json::to_value(reply).unwrap()).unwrap(),
            bytes
        );
    }
    let bytes = br#"{"generation":4,"notes":["owner note","removed tool"]}"#;
    let pending: Pending = serde_json::from_slice(bytes).unwrap();
    assert_eq!(serde_json::to_vec(&pending).unwrap(), bytes);
    let mut wire = view();
    wire["pending"] = json!({"generation":4});
    assert!(serde_json::from_value::<PlanView>(wire).is_ok());
    for pending in [
        json!({"generation":4,"notes":null}),
        json!({"generation":4,"notes":[]}),
        json!({"generation":4,"notes":[""]}),
        json!({"generation":4,"notes":["valid", ""]}),
        json!({"generation":4,"note":"legacy"}),
    ] {
        let mut wire = view();
        wire["pending"] = pending.clone();
        assert!(
            serde_json::from_value::<PlanView>(wire).is_err(),
            "{pending}"
        );
    }
}

#[test]
fn plan_view_refuses_invalid_utf8_ranges_order_and_overlap() {
    for ranges in [
        json!([[0, 0]]),
        json!([[2, 0]]),
        json!([[0, 10]]),
        json!([[1, 2]]),
        json!([[0, 1]]),
        json!([[3, 5]]),
        json!([[0, 6], [3, 6]]),
        json!([[7, 9], [0, 2]]),
        json!([[0, u64::MAX]]),
        json!([[u64::MAX, u64::MAX]]),
    ] {
        let mut wire = view();
        wire["system"]["items"] = Value::Array(
            ranges
                .as_array()
                .unwrap()
                .iter()
                .map(|range| json!({"provider":"p", "start":range[0], "end":range[1]}))
                .collect(),
        );
        let error = serde_json::from_value::<PlanView>(wire).unwrap_err();
        assert!(
            error.to_string().contains("system.items"),
            "{ranges}: {error}"
        );
    }
    let mut wire = view();
    wire["system"]["items"] = json!([]);
    assert!(serde_json::from_value::<PlanView>(wire).is_ok());
}

#[test]
fn plan_view_requires_as_planned_true_and_newer_pending() {
    for flag in [json!(false), json!(1), json!(null)] {
        let mut wire = view();
        wire["as_planned"] = flag;
        assert!(serde_json::from_value::<PlanView>(wire)
            .unwrap_err()
            .to_string()
            .contains("as_planned"));
    }
    let mut wire = view();
    wire.as_object_mut().unwrap().remove("as_planned");
    assert!(serde_json::from_value::<PlanView>(wire).is_err());
    for generation in [0, 2, 3] {
        let mut wire = view();
        wire["pending"]["generation"] = json!(generation);
        assert!(serde_json::from_value::<PlanView>(wire)
            .unwrap_err()
            .to_string()
            .contains("pending.generation"));
    }
}

#[test]
fn plan_view_reply_and_nested_objects_are_closed() {
    for path in [
        "",
        "/system",
        "/system/items/0",
        "/tools/0",
        "/tools/0/tokens",
        "/pending",
        "/newest_step",
    ] {
        let mut wire = view();
        wire.pointer_mut(path)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .insert("unknown".into(), json!(true));
        assert!(serde_json::from_value::<PlanView>(wire).is_err(), "{path}");
    }
    let mut wire = view();
    wire["tools"][0]["input_schema"] = json!({"arbitrary":[{"unknown":true}]});
    assert!(serde_json::from_value::<PlanView>(wire).is_ok());
    for key in ["generation", "system", "tools"] {
        let mut wire = view();
        wire.as_object_mut().unwrap().remove(key);
        assert!(serde_json::from_value::<PlanView>(wire).is_err(), "{key}");
    }
}

#[test]
fn plan_view_uses_session_read_admission() {
    let plan = json!({"readers":["reader"], "compaction_item":{"provider":"compactor"}})
        .as_object()
        .unwrap()
        .clone();
    let pending = json!({"readers":["next-reader"]})
        .as_object()
        .unwrap()
        .clone();
    for caller in ["owner", "reader", "next-reader", "compactor", "stranger"] {
        for pending in [None, Some(&pending)] {
            for local in [false, true] {
                let facts = ReadAdmission::new(false).with_local_operator(local);
                assert_eq!(
                    admit(
                        &plan,
                        pending,
                        caller,
                        "owner",
                        ops::SESSION_PLAN_VIEW,
                        facts
                    ),
                    admit(&plan, pending, caller, "owner", ops::SESSION_READ, facts),
                    "caller {caller}, local {local}"
                );
            }
        }
    }
    for current in [false, true] {
        assert_eq!(
            admit(
                &Default::default(),
                None,
                "caller",
                "owner",
                ops::SESSION_PLAN_VIEW,
                ReadAdmission::new(current)
            ),
            Ok(current)
        );
    }
}

#[test]
fn plan_view_describe_group_requires_its_read_operation() {
    let mut describe = RoleDescribe::new(
        vec![Major::new(
            PROVIDES,
            REQUIRED_OPS.iter().map(|op| (*op).into()).collect(),
            "alpha",
        )],
        "test",
    )
    .with_session_capabilities_from("baseline")
    .with_capabilities(vec![capabilities::PLAN_VIEW.into()]);
    assert!(check_describe(&serde_json::to_value(&describe).unwrap())
        .unwrap_err()
        .contains(&DescribeProblem::GroupIncomplete {
            capability: capabilities::PLAN_VIEW,
            op: ops::SESSION_PLAN_VIEW
        }));
    describe.majors[0].ops.push(ops::SESSION_PLAN_VIEW.into());
    assert!(check_describe(&serde_json::to_value(describe).unwrap()).is_ok());
}
