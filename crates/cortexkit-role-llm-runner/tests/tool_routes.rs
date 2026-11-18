use cortexkit_role_llm_runner::{
    read::ToolCallAttribution,
    tool_calls::RunToolCall,
    tool_routes::{self, ToolRoutes},
};
use serde_json::{json, Value};

fn samples() -> Value {
    serde_json::from_str(include_str!(
        "../test-vectors/llm-runner-v1/tool-routes.json"
    ))
    .unwrap()
}

#[test]
fn tool_routes_admission_refuses_every_shape_and_schema_clash() {
    let samples = samples();
    let valid = &samples["plan"];
    tool_routes::check_plan(valid, true).unwrap();
    let field = "plan.tool_items[0].routes";
    assert_eq!(tool_routes::check_plan(valid, false), Err(field.into()));
    for sample in samples["refused"].as_array().unwrap() {
        let mut plan = valid.clone();
        let routes = plan["tool_items"][0]["routes"].as_object_mut().unwrap();
        if let Some(patch) = sample["patch"].as_object() {
            routes.extend(patch.clone());
        }
        if let Some(remove) = sample["remove"].as_str() {
            routes.remove(remove);
        }
        assert_eq!(
            tool_routes::check_plan(&plan, true),
            Err(field.into()),
            "{}",
            sample["rule"]
        );
    }
    for bad in [Value::Null, json!(false), json!([])] {
        let mut plan = valid.clone();
        plan["tool_items"][0]["routes"] = bad;
        assert_eq!(tool_routes::check_plan(&plan, true), Err(field.into()));
    }
    let routes: ToolRoutes =
        serde_json::from_value(valid["tool_items"][0]["routes"].clone()).unwrap();
    assert_eq!(
        routes.check_schema(&samples["clash"]["input_schema"], 0),
        Err(field.into())
    );
    routes.check_schema(&samples["input_schema"], 0).unwrap();
    let mut boundary = routes.clone();
    boundary.argument = format!("a{}", "_".repeat(31));
    boundary.description = "é".repeat(256);
    boundary.values[0].value = "é".repeat(32);
    assert!(boundary.valid());
    boundary.description.push('x');
    assert!(!boundary.valid());
}

#[test]
fn tool_routes_offered_definition_and_stripped_dispatch_are_pinned() {
    let samples = samples();
    let pin = cortexkit_role_tool_provider::call::SchemaPin::parse(
        samples["dispatch"]["schema_pin"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(
        pin.schema_digest,
        cortexkit_role_tool_provider::catalog::schema_digest(&samples["input_schema"]).unwrap()
    );
    let routes: ToolRoutes =
        serde_json::from_value(samples["plan"]["tool_items"][0]["routes"].clone()).unwrap();
    assert_eq!(
        routes.offer(&samples["input_schema"]),
        samples["offered_schema"]
    );
    let original = samples["model_arguments"].clone();
    let (target, stripped, fact) = routes.dispatch("aft", &original).unwrap();
    assert_eq!(json!(target), samples["dispatch"]["provider"]);
    assert_eq!(stripped, samples["dispatch"]["arguments"]);
    assert_eq!(
        serde_json::to_value(&fact).unwrap(),
        samples["read"]["tool_route"]
    );
    assert_eq!(original["machine"], "laptop");
    let (target, args, fact) = routes
        .dispatch("aft", &json!({"path":"notes.txt"}))
        .unwrap();
    assert_eq!(target, "aft");
    assert_eq!(args, json!({"path":"notes.txt"}));
    assert!(fact.is_none());
    assert!(routes
        .dispatch("aft", &samples["invalid_call"]["arguments"])
        .is_err());
    assert!(routes.dispatch("aft", &json!({"machine":null})).is_err());
    let read: ToolCallAttribution = serde_json::from_value(samples["read"].clone()).unwrap();
    assert_eq!(serde_json::to_value(&read).unwrap(), samples["read"]);
    let call = RunToolCall::new("model-call-1", "read", stripped, "succeeded", 1)
        .with_dispatched_to(read.dispatched_to.unwrap())
        .with_tool_route(read.tool_route.unwrap());
    let encoded = serde_json::to_value(&call).unwrap();
    assert_eq!(encoded["tool_route"], samples["read"]["tool_route"]);
    assert_eq!(
        serde_json::from_value::<RunToolCall>(encoded).unwrap(),
        call
    );
    assert!(serde_json::to_value(ToolCallAttribution::new("default"))
        .unwrap()
        .get("tool_route")
        .is_none());
}
