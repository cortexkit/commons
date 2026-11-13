//! Validate plan-carried rendering facts without rewriting their JSON.

use serde_json::Value;

pub use cortexkit_role_tool_provider::SystemTextFacts;

/// The provider manifest token, not a runner capability inferred from its ops.
pub use cortexkit_role_tool_provider::SYSTEM_TEXT_FACTS_V1;

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
