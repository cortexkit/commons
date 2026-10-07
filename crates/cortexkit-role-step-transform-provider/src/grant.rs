//! The user tier's grant to `replace` on `post_tool`.
//!
//! Replacing a tool's result is a permission only the user tier gives, per
//! provider and hook, as `{module, hook, tools}` ([`UserGrant`]). Neither
//! project configuration nor a provider can grant it, and no provider has it
//! by default.
//!
//! The grant travels in the plan, in one dedicated top-level field,
//! [`USER_GRANTS_FIELD`], which the plan composer fills only from the user
//! tier, and is frozen with the plan at admission. Nothing else carries it:
//! a grant-shaped value in any other plan field (an item's params, a
//! subscription, a misspelled field) is ordinary data and grants nothing,
//! so ordinary plan data can never manufacture user permission.
//! [`plan_user_grants`] reads the one field and nothing else.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::subscription::Hook;

/// The plan field that carries the user tier's grants, and the only one.
pub const USER_GRANTS_FIELD: &str = "user_grants";

/// One user-tier grant: `module` may `replace` on `hook` for the listed
/// `tools`. Only `post_tool` grants mean anything today; a grant on another
/// hook allows nothing.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct UserGrant {
    /// The provider module granted the permission.
    pub module: String,
    pub hook: Hook,
    /// The tools whose results it may replace. An empty list allows none.
    pub tools: Vec<String>,
}

impl UserGrant {
    pub fn new(module: impl Into<String>, hook: Hook, tools: Vec<String>) -> Self {
        Self {
            module: module.into(),
            hook,
            tools,
        }
    }

    /// Whether this grant lets `module` replace the result of `tool`.
    pub fn allows_post_tool_replace(&self, module: &str, tool: &str) -> bool {
        self.hook == Hook::PostTool
            && self.module == module
            && self.tools.iter().any(|granted| granted == tool)
    }
}

/// The grants of a plan: the array at [`USER_GRANTS_FIELD`], or none when
/// the field is absent. No other field is read, so a grant placed anywhere
/// else is ignored. A present but malformed field is an error, which the
/// runner refuses at admission as `invalid_params {field: "plan.user_grants"}`.
pub fn plan_user_grants(plan: &Map<String, Value>) -> Result<Vec<UserGrant>, serde_json::Error> {
    match plan.get(USER_GRANTS_FIELD) {
        None => Ok(Vec::new()),
        Some(grants) => serde_json::from_value(grants.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn grant_vectors_read_only_the_dedicated_field() {
        let file = vectors::load("grants.json");
        for case in vectors::cases(&file, "plans") {
            let name = case["name"].as_str().unwrap();
            let plan = case["plan"].as_object().unwrap();
            let grants = plan_user_grants(plan).unwrap_or_else(|e| panic!("{name}: {e}"));
            if let Some(field) = plan.get(USER_GRANTS_FIELD) {
                vectors::round_trip::<Vec<UserGrant>>(name, field);
            }
            for check in case["checks"].as_array().unwrap() {
                let module = check["module"].as_str().unwrap();
                let tool = check["tool"].as_str().unwrap();
                assert_eq!(
                    grants
                        .iter()
                        .any(|grant| grant.allows_post_tool_replace(module, tool)),
                    check["allowed"].as_bool().unwrap(),
                    "{name}: {module} on {tool}"
                );
            }
        }
        for case in vectors::cases(&file, "malformed") {
            let name = case["name"].as_str().unwrap();
            assert!(
                plan_user_grants(case["plan"].as_object().unwrap()).is_err(),
                "{name}"
            );
        }
    }
}
