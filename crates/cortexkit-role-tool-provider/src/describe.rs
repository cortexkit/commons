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

use crate::{ops, PROVIDES, REQUIRED_OPS};

/// The `role.describe` answer: every major of the role the module serves,
/// each with its own ops and stability. The caller picks the highest major it
/// understands for a new session.
///
/// Decoded leniently: unknown fields, majors, ops and capabilities are
/// ignored. Only the presence of the `tool-provider/v1` major and its required
/// ops are checked strictly, by [`check_describe`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct RoleDescribe {
    pub majors: Vec<Major>,
    /// The module's own build identity, for display. Never a cache key.
    pub implementation_version: String,
    /// Module-level capabilities. `tool-provider/v1` defines none; session
    /// capabilities travel in the catalog answer instead.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// One served major.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Major {
    /// The role and major as the manifest spells it, for example
    /// `tool-provider/v1`.
    pub version: String,
    pub ops: Vec<String>,
    /// `alpha`, `beta` or `stable`; see [`Major::stability`].
    pub stability: String,
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
    /// The entry for `version` (for example `tool-provider/v1`), if served.
    pub fn major(&self, version: &str) -> Option<&Major> {
        self.majors.iter().find(|major| major.version == version)
    }
}

impl Major {
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

    /// Whether the module can hold a call past its reply under this major,
    /// and so serves `tool.withdraw`.
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
    /// No entry in `majors` is `tool-provider/v1`.
    MissingMajor { found: Vec<String> },
    /// The `tool-provider/v1` entry lacks a required op.
    MissingOp(&'static str),
    /// The answer carries a `tools` list. The catalog comes only from
    /// `tool.catalog`, so a describe answer stays cacheable.
    CarriesToolList,
}

/// Decode `raw` and check it lists a `tool-provider/v1` major with every
/// required op. Every problem is returned, not only the first. On success
/// the answer's `tool-provider/v1` entry is [`RoleDescribe::major`] of
/// [`PROVIDES`].
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
    match describe.major(PROVIDES) {
        None => problems.push(DescribeProblem::MissingMajor {
            found: describe.majors.iter().map(|m| m.version.clone()).collect(),
        }),
        Some(major) => {
            for op in REQUIRED_OPS {
                if !major.serves(op) {
                    problems.push(DescribeProblem::MissingOp(op));
                }
            }
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
                    DescribeProblem::MissingMajor { .. } => "missing_major",
                    DescribeProblem::MissingOp(_) => "missing_op",
                    DescribeProblem::CarriesToolList => "carries_tool_list",
                })
                .collect();
            assert!(names.contains(&expected), "{}: {names:?}", case["name"]);
        }
    }

    #[test]
    fn unknown_stability_extra_majors_and_extra_ops_are_tolerated() {
        let answer = serde_json::json!({
            "majors": [
                {"version": "tool-provider/v1", "stability": "experimental",
                 "ops": ["role.describe", "tool.catalog", "vendor.extra"]},
                {"version": "tool-provider/v2", "stability": "alpha", "ops": ["role.describe"]}
            ],
            "implementation_version": "1.2.3",
            "future_field": {}
        });
        let describe = check_describe(&answer).unwrap();
        let v1 = describe.major(PROVIDES).unwrap();
        assert_eq!(v1.stability(), Stability::Other("experimental".into()));
        assert!(!v1.holds_calls());
        assert_eq!(
            describe.major("tool-provider/v2").unwrap().stability(),
            Stability::Alpha
        );
    }
}
