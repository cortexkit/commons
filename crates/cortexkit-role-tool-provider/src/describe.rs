//! `role.describe`: discovery before use.
//!
//! A consumer that finds a module claiming `tool-provider/v1` calls
//! `role.describe` and refuses the module by name if a required op is
//! missing, before routing anything to it. The answer describes the role
//! implementation, never the tools: the catalog is `tool.catalog`'s job. It
//! depends only on the module build, so a consumer may cache it for as long
//! as it talks to the same module incarnation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ops, REQUIRED_OPS, ROLE, VERSION};

/// The `role.describe` answer.
///
/// Decoded leniently: unknown fields, ops and capabilities are ignored. Only
/// the role, the listed versions and the presence of the required ops are checked
/// strictly, by [`check_describe`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RoleDescribe {
    pub role: String,
    /// Every major of the role the module serves, for example `["v1"]`. The
    /// caller picks the highest it understands for a new session.
    pub versions: Vec<String>,
    /// `alpha`, `beta` or `stable`; see [`RoleDescribe::stability`].
    pub stability: String,
    /// The module's own build identity, for display. Never a cache key.
    pub implementation_version: String,
    pub ops: Vec<String>,
    /// Module-level capabilities. `tool-provider/v1` defines none; session
    /// capabilities travel in the catalog answer instead.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// A stability level. Unknown levels decode as [`Stability::Other`] rather
/// than failing the answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stability {
    Alpha,
    Beta,
    Stable,
    Other(String),
}

impl RoleDescribe {
    pub fn stability(&self) -> Stability {
        match self.stability.as_str() {
            "alpha" => Stability::Alpha,
            "beta" => Stability::Beta,
            "stable" => Stability::Stable,
            other => Stability::Other(other.to_owned()),
        }
    }

    pub fn serves(&self, op: &str) -> bool {
        self.ops.iter().any(|served| served == op)
    }

    /// Whether the module can hold a call past its reply, and so serves
    /// `tool.withdraw`.
    pub fn holds_calls(&self) -> bool {
        self.serves(ops::TOOL_WITHDRAW)
    }
}

/// Why a `role.describe` answer does not describe a `tool-provider/v1`
/// module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescribeProblem {
    /// The answer is not a JSON object.
    NotAnObject,
    /// A required field is missing or has the wrong type.
    Undecodable(String),
    WrongRole {
        found: String,
    },
    /// `versions` does not list `v1`.
    MissingVersion {
        found: Vec<String>,
    },
    MissingOp(&'static str),
    /// The answer carries a `tools` list. The catalog comes only from
    /// `tool.catalog`, so a describe answer stays cacheable.
    CarriesToolList,
}

/// Decode `raw` and check it describes a module serving `tool-provider/v1`, with every
/// required op. Every problem is returned, not only the first.
pub fn check_describe(raw: &Value) -> Result<RoleDescribe, Vec<DescribeProblem>> {
    let Some(object) = raw.as_object() else {
        return Err(vec![DescribeProblem::NotAnObject]);
    };
    let mut problems = Vec::new();
    if object.contains_key("tools") {
        problems.push(DescribeProblem::CarriesToolList);
    }
    let describe: RoleDescribe = match serde_json::from_value(raw.clone()) {
        Ok(describe) => describe,
        Err(error) => {
            problems.push(DescribeProblem::Undecodable(error.to_string()));
            return Err(problems);
        }
    };
    if describe.role != ROLE {
        problems.push(DescribeProblem::WrongRole {
            found: describe.role.clone(),
        });
    }
    if !describe.versions.iter().any(|version| version == VERSION) {
        problems.push(DescribeProblem::MissingVersion {
            found: describe.versions.clone(),
        });
    }
    for op in REQUIRED_OPS {
        if !describe.serves(op) {
            problems.push(DescribeProblem::MissingOp(op));
        }
    }
    if problems.is_empty() {
        Ok(describe)
    } else {
        Err(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn describe_vectors_check_as_recorded() {
        let vectors = vectors::load("role-describe.json");
        for case in vectors["accepted"].as_array().unwrap() {
            let answer = &case["answer"];
            assert!(
                check_describe(answer).is_ok(),
                "{}: {:?}",
                case["name"],
                check_describe(answer)
            );
        }
        for case in vectors["refused"].as_array().unwrap() {
            let problems =
                check_describe(&case["answer"]).expect_err(case["name"].as_str().unwrap());
            let expected = case["problem"].as_str().unwrap();
            let names: Vec<&str> = problems
                .iter()
                .map(|problem| match problem {
                    DescribeProblem::NotAnObject => "not_an_object",
                    DescribeProblem::Undecodable(_) => "undecodable",
                    DescribeProblem::WrongRole { .. } => "wrong_role",
                    DescribeProblem::MissingVersion { .. } => "missing_version",
                    DescribeProblem::MissingOp(_) => "missing_op",
                    DescribeProblem::CarriesToolList => "carries_tool_list",
                })
                .collect();
            assert!(names.contains(&expected), "{}: {names:?}", case["name"]);
        }
    }

    #[test]
    fn unknown_stability_and_extra_ops_are_tolerated() {
        let answer = serde_json::json!({
            "role": "tool-provider", "versions": ["v1", "v2"], "stability": "experimental",
            "implementation_version": "1.2.3",
            "ops": ["role.describe", "tool.catalog", "vendor.extra"],
            "future_field": {}
        });
        let describe = check_describe(&answer).unwrap();
        assert_eq!(
            describe.stability(),
            Stability::Other("experimental".into())
        );
        assert!(!describe.holds_calls());
    }
}
