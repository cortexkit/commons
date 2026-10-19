//! The `compaction` group's admission check, which keeps a compaction
//! provider off a runner that cannot host one.
//!
//! The compaction interface itself (Setup, advance notices, views, rides,
//! the ceiling and native markers) is the runner half of
//! compaction-provider 0.3.0 §3, whose role defines its shapes. A runner owes
//! it only if it declares the `compaction` group, which requires `tags`
//! (CONTRACT §3, §11.1). The runner serves no compaction op of its own.

use serde_json::{Map, Value};

use crate::capabilities;

/// The plan member that names the session's compaction provider, and the
/// `field` an `invalid_params` refusal names when a plan carries it to a
/// runner that does not declare `compaction`.
pub const PLAN_COMPACTION_ITEM: &str = "compaction_item";

/// The `field` of that admission refusal: the plan's `compaction_item`.
pub const PLAN_COMPACTION_ITEM_FIELD: &str = "plan.compaction_item";

/// The provider a plan's `compaction_item` names, or `None` when the plan
/// has no `compaction_item` or it names no provider.
pub fn plan_compaction_provider(plan: &Map<String, Value>) -> Option<&str> {
    plan.get(PLAN_COMPACTION_ITEM)?.get("provider")?.as_str()
}

/// Admission's check of a plan against the runner's module-level
/// capabilities: a plan carrying a `compaction_item` (any value but `null`)
/// is refused `invalid_params` naming [`PLAN_COMPACTION_ITEM_FIELD`] unless
/// `declared` includes `compaction`. A runner without the group never calls
/// a compaction provider, so admitting the plan would leave the session's
/// compaction owner uncalled. The refusal writes nothing.
pub fn check_plan_compaction<S: AsRef<str>>(
    plan: &Map<String, Value>,
    declared: &[S],
) -> Result<(), &'static str> {
    let carries_item = plan
        .get(PLAN_COMPACTION_ITEM)
        .is_some_and(|item| !item.is_null());
    let declares_group = declared
        .iter()
        .any(|name| name.as_ref() == capabilities::COMPACTION);
    if carries_item && !declares_group {
        Err(PLAN_COMPACTION_ITEM_FIELD)
    } else {
        Ok(())
    }
}
