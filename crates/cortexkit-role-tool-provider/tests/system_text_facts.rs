use cortexkit_role_tool_provider::catalog::CatalogRequest;
use serde_json::{json, Value};

fn facts() -> Value {
    json!({"model":{"provider":"vendor","id":"model"},"harness":{"id":"host","facts":["files","shell"]},"tools":["read","write"],"idle_start":["sender.a","sender.b"]})
}

#[test]
fn system_text_facts_request_preserves_facts() {
    let wire = json!({"params":{},"system_text":{"params":{}},"facts":facts()});
    let request: CatalogRequest = serde_json::from_value(wire).unwrap();
    assert_eq!(serde_json::to_value(request).unwrap()["facts"], facts());
}

#[test]
fn system_text_facts_refuses_unsorted_lists() {
    for path in ["tools", "idle_start", "harness"] {
        let mut value = facts();
        let list = if path == "harness" {
            &mut value[path]["facts"]
        } else {
            &mut value[path]
        };
        *list = json!(["z", "a"]);
        assert!(
            serde_json::from_value::<CatalogRequest>(json!({"facts":value})).is_err(),
            "{path}"
        );
    }
}

#[test]
fn system_text_facts_refuses_duplicate_lists() {
    for path in ["tools", "idle_start", "harness"] {
        let mut value = facts();
        let list = if path == "harness" {
            &mut value[path]["facts"]
        } else {
            &mut value[path]
        };
        *list = json!(["a", "a"]);
        assert!(
            serde_json::from_value::<CatalogRequest>(json!({"facts":value})).is_err(),
            "{path}"
        );
    }
}

#[test]
fn system_text_facts_refuses_empty_and_null_names() {
    for path in ["tools", "idle_start", "harness"] {
        for invalid in [json!([""]), json!([null]), Value::Null] {
            let mut value = facts();
            let list = if path == "harness" {
                &mut value[path]["facts"]
            } else {
                &mut value[path]
            };
            *list = invalid;
            assert!(
                serde_json::from_value::<CatalogRequest>(json!({"facts":value})).is_err(),
                "{path}"
            );
        }
    }
}

#[test]
fn system_text_facts_refuses_wrong_shape() {
    for value in [
        Value::Null,
        json!({}),
        json!([]),
        json!({"model":null,"harness":{},"tools":[],"idle_start":[]}),
    ] {
        assert!(
            serde_json::from_value::<CatalogRequest>(json!({"facts":value})).is_err(),
            "{value}"
        );
    }
}

#[test]
fn system_text_facts_refuses_extra_state() {
    for key in ["variant", "effort", "time", "count", "session"] {
        let mut value = facts();
        value[key] = json!("unplanned");
        assert!(
            serde_json::from_value::<CatalogRequest>(json!({"facts":value})).is_err(),
            "{key}"
        );
    }
    for key in ["model", "harness"] {
        let mut value = facts();
        value[key]["extra"] = json!(true);
        assert!(
            serde_json::from_value::<CatalogRequest>(json!({"facts":value})).is_err(),
            "{key}"
        );
    }
}

#[test]
fn system_text_facts_schema_matches_structural_decode() {
    use cortexkit_role_tool_provider::{SystemTextFacts, SYSTEM_TEXT_FACTS_SCHEMA};
    assert_eq!(
        SYSTEM_TEXT_FACTS_SCHEMA,
        include_str!("../system-text-facts.schema.json")
    );
    let schema: Value = serde_json::from_str(SYSTEM_TEXT_FACTS_SCHEMA).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let mut samples = vec![
        facts(),
        json!({"model":{"provider":"","id":""},"harness":{"id":"","facts":[]},"tools":[],"idle_start":[]}),
        Value::Null,
        json!([]),
        json!(false),
    ];
    for key in ["model", "harness", "tools", "idle_start"] {
        let mut missing = facts();
        missing.as_object_mut().unwrap().remove(key);
        samples.push(missing);
        for invalid in [Value::Null, json!(1), json!([null])] {
            let mut sample = facts();
            sample[key] = invalid;
            samples.push(sample);
        }
    }
    for (parent, keys) in [
        ("model", vec!["id", "provider"]),
        ("harness", vec!["id", "facts"]),
    ] {
        for key in keys {
            let mut missing = facts();
            missing[parent].as_object_mut().unwrap().remove(key);
            samples.push(missing);
            for invalid in [Value::Null, json!(1)] {
                let mut sample = facts();
                sample[parent][key] = invalid;
                samples.push(sample);
            }
        }
        let mut extra = facts();
        extra[parent]["extra"] = json!(true);
        samples.push(extra);
    }
    for path in ["tools", "idle_start", "harness"] {
        for invalid in [json!([""]), json!(["a", "a"]), json!([null])] {
            let mut sample = facts();
            if path == "harness" {
                sample[path]["facts"] = invalid;
            } else {
                sample[path] = invalid;
            }
            samples.push(sample);
        }
    }
    let mut extra = facts();
    extra["session"] = json!("forbidden");
    samples.push(extra);
    for sample in samples {
        assert_eq!(
            validator.is_valid(&sample),
            serde_json::from_value::<SystemTextFacts>(sample.clone()).is_ok(),
            "{sample}"
        );
    }
}

#[test]
fn system_text_facts_canonical_bytes_and_capability_gating() {
    use cortexkit_role_tool_provider::{
        SystemTextFacts, SystemTextHarness, SystemTextModel, SYSTEM_TEXT_FACTS_V1,
    };
    let value = SystemTextFacts::new(
        SystemTextModel::new("vendor", "model"),
        SystemTextHarness::new("host", vec!["shell".into(), "files".into(), "files".into()])
            .unwrap(),
        vec!["write".into(), "read".into(), "write".into()],
        vec!["sender.b".into(), "sender.a".into()],
    )
    .unwrap();
    assert_eq!(value.canonical_json().unwrap(), br#"{"harness":{"facts":["files","shell"],"id":"host"},"idle_start":["sender.a","sender.b"],"model":{"id":"model","provider":"vendor"},"tools":["read","write"]}"#);
    let unicode = SystemTextFacts::new(
        SystemTextModel::new("vendor", "model"),
        SystemTextHarness::new("host", vec![]).unwrap(),
        vec!["𐀀".into(), "\u{e000}".into(), "a".into()],
        vec![],
    )
    .unwrap();
    assert_eq!(unicode.tools(), ["a", "\u{e000}", "𐀀"]);
    assert!(SystemTextHarness::new("host", vec!["".into()]).is_err());
    assert!(SystemTextFacts::new(
        value.model().clone(),
        value.harness().clone(),
        vec!["".into()],
        vec![]
    )
    .is_err());
    let request = CatalogRequest::new(Default::default())
        .with_system_text_facts(value.clone(), &[SYSTEM_TEXT_FACTS_V1.into()]);
    assert_eq!(request.facts.as_ref(), Some(&value));
    for provides in [vec![], vec!["system-text-facts/v2".into()]] {
        let legacy = request
            .clone()
            .with_system_text_facts(value.clone(), &provides);
        assert!(serde_json::to_value(legacy).unwrap().get("facts").is_none());
    }
}
