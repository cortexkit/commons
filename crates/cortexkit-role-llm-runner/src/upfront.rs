//! Plan-item rendering hints for hosts that defer tool definitions (CONTRACT
//! §9.18).
//!
//! These helpers only check values the runner passes in; the ordering is the
//! runner's job. Call [`check_plan`] before fetching or writing anything. After
//! fetching an item and applying its exclusions, call [`resolve`] with that
//! item's remaining offered names, and skip it for an optional item that did
//! not arrive. Neither helper builds a provider request: keeping `upfront` out
//! of what is forwarded to the provider is also the runner's job.

use std::collections::BTreeSet;

use serde_json::{json, Value};

/// The `plan_stale` difference kind for an upfront name the item does not offer.
pub const DIFFERENCE_KIND: &str = "upfront";

/// Check every present `upfront` member, even on optional items. A member is
/// valid only when `declared` (the caller's own `tool_upfront` declaration) is
/// true and the member is a non-empty array of distinct, non-empty strings.
/// On failure, the returned string is the `field` for the runner's
/// `invalid_params` refusal; the runner builds that refusal. An absent member
/// passes. A plan without an `items` array passes too: this checks only the
/// `upfront` members, and the runner's ordinary plan admission checks the rest.
pub fn check_plan(plan: &Value, declared: bool) -> Result<(), String> {
    if let Some(items) = plan.get("items").and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            if let Some(value) = item.get("upfront") {
                let mut names = BTreeSet::new();
                let valid = value.as_array().is_some_and(|values| {
                    !values.is_empty()
                        && values.iter().all(|value| {
                            value
                                .as_str()
                                .is_some_and(|name| !name.is_empty() && names.insert(name))
                        })
                });
                if !declared || !valid {
                    return Err(format!("plan.items[{index}].upfront"));
                }
            }
        }
    }
    Ok(())
}

/// One item's upfront names that are not among the tools it offers once its
/// exclusions apply. The runner refuses the plan, before any write, with
/// [`Self::code`] (`plan_stale`) and [`Self::detail`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct UpfrontStale {
    pub provider: String,
    pub names: Vec<String>,
}

impl UpfrontStale {
    /// The refusal code, shared with other stale-plan differences.
    pub fn code(&self) -> &'static str {
        crate::errors::PLAN_STALE
    }

    /// `{differences: [{kind: "upfront", provider, name}, ...]}` in list order.
    pub fn detail(&self) -> Value {
        let differences: Vec<Value> = self
            .names
            .iter()
            .map(|name| json!({"kind": DIFFERENCE_KIND, "provider": self.provider, "name": name}))
            .collect();
        json!({"differences": differences})
    }
}

/// Compare one arrived item's `upfront` list, already checked by [`check_plan`],
/// with `offered`: the bare tool names that item offers once its exclusions
/// apply. The caller fetches the item and applies the exclusions; this only
/// compares names. On success return the unchanged list, not a replacement
/// offered set. On failure return every missing name, in list order. Do not
/// call it for an optional item that did not arrive.
pub fn resolve<'a>(
    provider: &str,
    upfront: &[String],
    offered: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<String>, UpfrontStale> {
    let offered: BTreeSet<&str> = offered.into_iter().collect();
    let names: Vec<String> = upfront
        .iter()
        .filter(|name| !offered.contains(name.as_str()))
        .cloned()
        .collect();
    if names.is_empty() {
        Ok(upfront.to_vec())
    } else {
        Err(UpfrontStale {
            provider: provider.to_owned(),
            names,
        })
    }
}
