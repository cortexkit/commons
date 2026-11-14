//! Validate plan-carried rendering facts without rewriting their JSON.

use serde_json::Value;

pub use cortexkit_role_tool_provider::SystemTextFacts;

/// The provider manifest token, not a runner capability inferred from its ops.
pub use cortexkit_role_tool_provider::SYSTEM_TEXT_FACTS_V1;

/// Prepare a composer's plan for a runner's checked describe answer. Keep
/// system-text item facts unchanged only when the runner declares `plan_facts`;
/// otherwise remove the member, since older runners may reject unknown fields.
/// All other plan fields are left unchanged. This does not validate the plan.
pub fn for_runner(plan: &mut Value, runner: &crate::describe::RoleDescribe) {
    if runner.declares(crate::capabilities::PLAN_FACTS) {
        return;
    }
    if let Some(items) = plan
        .get_mut("system_text_items")
        .and_then(Value::as_array_mut)
    {
        for item in items {
            if let Some(item) = item.as_object_mut() {
                item.remove("facts");
            }
        }
    }
}

/// Validate an optional `facts` member on a system-text plan item. The plan is
/// otherwise opaque; the returned reference is the original JSON, not a
/// serialization of the decoded type. Copy it to `tool.catalog.facts` unchanged.
/// A non-declaring provider never receives the member, even when the plan has
/// it. A malformed present member is refused before any fetch or state write.
/// The error string is the field for the runner's `invalid_params` refusal.
pub fn for_provider<'a>(item: &'a Value, provides: &[String]) -> Result<Option<&'a Value>, String> {
    let Some(facts) = item.get("facts") else {
        return Ok(None);
    };
    serde_json::from_value::<SystemTextFacts>(facts.clone()).map_err(|_| "facts".to_owned())?;
    Ok(provides
        .iter()
        .any(|token| token == SYSTEM_TEXT_FACTS_V1)
        .then_some(facts))
}
