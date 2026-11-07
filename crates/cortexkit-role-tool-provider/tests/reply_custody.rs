use cortexkit_role_tool_provider::{
    catalog::CatalogTool,
    reply_custody::{self, ExecutionMode},
    vectors,
};
use serde_json::{json, Value};

fn samples() -> Value {
    serde_json::from_str(vectors::REPLY_CUSTODY_JSON).unwrap()
}

#[test]
fn reply_custody_catalog_execution_modes_round_trip_without_adding_absent_bytes() {
    for sample in samples()["catalogs"].as_array().unwrap() {
        let tool: CatalogTool = serde_json::from_value(sample["tool"].clone()).unwrap();
        assert_eq!(serde_json::to_value(tool).unwrap(), sample["tool"]);
    }
}

#[test]
fn reply_custody_decisions_use_only_the_frozen_catalog() {
    for sample in samples()["decisions"].as_array().unwrap() {
        let mut tool = CatalogTool::new("work", "digest", 1, json!({"type":"object"}));
        tool.execution_mode = sample["mode"]
            .as_str()
            .map(|s| ExecutionMode::from(s.to_owned()));
        let opt_in = sample["opt_in"].as_bool().unwrap();
        let keyed = sample["keyed"].as_bool().unwrap();
        assert_eq!(
            reply_custody::entry_required(
                opt_in,
                keyed,
                sample["held"].as_bool().unwrap(),
                sample["direct_reply_sent"].as_bool().unwrap_or(true),
                &tool,
            ),
            sample["entry"].as_bool().unwrap(),
            "{sample}"
        );
        assert_eq!(
            reply_custody::survives_route_close(opt_in, keyed, &tool),
            sample["survives"].as_bool().unwrap(),
            "{sample}"
        );
    }
}

#[test]
fn reply_custody_dependency_refusals_name_the_missing_capability() {
    for sample in samples()["dependencies"].as_array().unwrap() {
        let checked = reply_custody::check_dependencies(
            sample["custody"].as_bool().unwrap(),
            sample["late"].as_bool().unwrap(),
            sample["key"].as_bool().unwrap(),
        );
        if let Some(missing) = sample["missing"].as_str() {
            let error = checked.unwrap_err();
            assert_eq!(error.code, "invalid_request");
            assert!(serde_json::to_string(&error).unwrap().contains(missing));
        } else {
            checked.unwrap();
        }
    }
}

#[test]
fn reply_custody_bind_only_opt_in() {
    assert!(!reply_custody::route_opts_in(None));
    assert!(!reply_custody::route_opts_in(Some(&[])));
    assert!(!reply_custody::route_opts_in(Some(
        &["late_results".into()]
    )));
    assert!(reply_custody::route_opts_in(Some(
        &["reply_custody".into()]
    )));
}
