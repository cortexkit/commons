//! Plan selection of a compaction provider. Protocol obligations belong to
//! `cortexkit-role-compaction` (`compaction-runner/v1`, `compaction-provider/v2`).

use serde_json::{Map, Value};

/// The plan member naming the provider, required session-role preset and parameters.
pub const PLAN_COMPACTION_ITEM: &str = "compaction_item";

/// The provider named by the plan, if any.
pub fn plan_compaction_provider(plan: &Map<String, Value>) -> Option<&str> {
    plan.get(PLAN_COMPACTION_ITEM)?.get("provider")?.as_str()
}

/// A provider plan must name the session's role explicitly, because
/// compaction-provider/v2 Setup requires that same preset. No runner default
/// can stand in for the plan composer's role choice.
pub fn check_plan(plan: &Map<String, Value>) -> Result<(), &'static str> {
    if let Some(item) = plan
        .get(PLAN_COMPACTION_ITEM)
        .filter(|item| !item.is_null())
    {
        if !item.get("preset").is_some_and(Value::is_string) {
            return Err("plan.compaction_item.preset");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn compaction_plan_requires_an_explicit_role_preset() {
        let vectors = crate::vectors::load("plan-compaction-preset.json");
        assert_eq!(check_plan(vectors["accepted"].as_object().unwrap()), Ok(()));
        for case in vectors["refused"].as_array().unwrap() {
            assert_eq!(
                check_plan(case["plan"].as_object().unwrap()),
                Err(case["error"]["detail"]["field"].as_str().unwrap())
            );
        }
        for item in [
            json!({"provider":"example.compactor"}),
            json!({"provider":"example.compactor","preset":null}),
        ] {
            assert_eq!(
                check_plan(json!({"compaction_item":item}).as_object().unwrap()),
                Err("plan.compaction_item.preset")
            );
        }
        let plan = json!({"compaction_item":{"provider":"example.compactor","preset":"worker","params":{}}});
        assert_eq!(check_plan(plan.as_object().unwrap()), Ok(()));
        assert_eq!(
            plan_compaction_provider(plan.as_object().unwrap()),
            Some("example.compactor")
        );
    }
}
