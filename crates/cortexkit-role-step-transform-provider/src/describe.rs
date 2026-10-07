//! `role.describe`: discovery before use.
//!
//! A runner that is handed a plan naming a step-transform provider calls
//! `role.describe` on it and refuses the provider by name if a required op
//! is missing, before routing anything to it. The answer describes the
//! provider build, never a session, so a consumer may cache it for as long
//! as it talks to the same module incarnation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{PROVIDES, REQUIRED_OPS};

/// Empty params for discovery. Unknown fields are ignored.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DescribeRequest {}

impl DescribeRequest {
    pub fn new() -> Self {
        Self::default()
    }
}

/// The `role.describe` answer: every major of the role the module serves,
/// each with its own ops and stability, plus module-level capabilities and
/// the runner groups the provider needs.
///
/// Decoded leniently: unknown fields, majors, ops and capabilities are
/// ignored. [`check_describe`] checks strictly only the
/// `step-transform-provider/v1` major and its required ops.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`RoleDescribe::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RoleDescribe {
    pub majors: Vec<Major>,
    /// The module's own build identity, for display. Never a cache key.
    pub implementation_version: String,
    /// Module-level capabilities. This role defines none yet; the member is
    /// here so one can be added without a new shape.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// The `llm-runner/v1` capability groups this provider needs from the
    /// runner, for example `transcript_reads` for a provider that reads
    /// history beyond what its hooks see. The plan composer checks them
    /// against the groups the runner advertises in its `role.describe` when
    /// it composes the session, and an unmet group fails the launch, named
    /// ([`RoleDescribe::unmet_runner_groups`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runner_groups: Vec<String>,
}

fn alpha() -> String {
    "alpha".to_owned()
}

/// One served major.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Major {
    /// The role and major as the manifest spells it, for example
    /// `step-transform-provider/v1`.
    pub version: String,
    pub ops: Vec<String>,
    /// `alpha`, `beta` or `stable`; see [`Major::stability`]. An answer that
    /// omits it is read as `alpha`, the level every major starts at.
    #[serde(default = "alpha")]
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
    /// An answer for the given majors and build identity, declaring no
    /// capability and needing no runner group.
    pub fn new(majors: Vec<Major>, implementation_version: impl Into<String>) -> Self {
        Self {
            majors,
            implementation_version: implementation_version.into(),
            ..Self::default()
        }
    }

    /// Set the runner groups the provider needs.
    pub fn with_runner_groups(mut self, groups: Vec<String>) -> Self {
        self.runner_groups = groups;
        self
    }

    /// The entry for `version` (for example `step-transform-provider/v1`), if
    /// served.
    pub fn major(&self, version: &str) -> Option<&Major> {
        self.majors.iter().find(|major| major.version == version)
    }

    /// The runner groups this provider needs that `declared` (a runner's
    /// `role.describe` capabilities) does not list, in the order the
    /// provider listed them. When this is not empty the composer fails the
    /// launch and names each group listed.
    pub fn unmet_runner_groups<'a>(&'a self, declared: &[String]) -> Vec<&'a str> {
        self.runner_groups
            .iter()
            .filter(|group| !declared.contains(group))
            .map(String::as_str)
            .collect()
    }
}

impl Major {
    pub fn new(version: impl Into<String>, ops: Vec<String>, stability: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            ops,
            stability: stability.into(),
        }
    }

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
}

/// Why a `role.describe` answer does not describe a usable
/// `step-transform-provider/v1` module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescribeProblem {
    /// The answer is not a JSON object.
    NotAnObject,
    /// A required field is missing or has the wrong type.
    Undecodable(String),
    /// No entry in `majors` is `step-transform-provider/v1`.
    MissingMajor { found: Vec<String> },
    /// The `step-transform-provider/v1` entry lacks a required op.
    MissingOp(&'static str),
}

impl DescribeProblem {
    /// The problem's name as the vectors spell it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::NotAnObject => "not_an_object",
            Self::Undecodable(_) => "undecodable",
            Self::MissingMajor { .. } => "missing_major",
            Self::MissingOp(_) => "missing_op",
        }
    }
}

/// Decode `raw` and check it lists a `step-transform-provider/v1` major with
/// every required op. Every problem is returned, not only the first.
pub fn check_describe(raw: &Value) -> Result<RoleDescribe, Vec<DescribeProblem>> {
    if !raw.is_object() {
        return Err(vec![DescribeProblem::NotAnObject]);
    }
    let describe: RoleDescribe = match serde_json::from_value(raw.clone()) {
        Ok(describe) => describe,
        Err(error) => return Err(vec![DescribeProblem::Undecodable(error.to_string())]),
    };
    let mut problems = Vec::new();
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
        let file = vectors::load("role-describe.json");
        for case in vectors::cases(&file, "canonical") {
            let name = case["name"].as_str().unwrap();
            vectors::round_trip::<RoleDescribe>(name, &case["answer"]);
        }
        for case in vectors::cases(&file, "accepted") {
            let name = case["name"].as_str().unwrap();
            let describe = check_describe(&case["answer"])
                .unwrap_or_else(|problems| panic!("{name}: {problems:?}"));
            let v1 = describe.major(PROVIDES).unwrap();
            assert_eq!(
                v1.stability,
                case["stability"].as_str().unwrap(),
                "{name}: stability"
            );
            if let Some(declared) = case.get("runner_declares") {
                let declared: Vec<String> = serde_json::from_value(declared.clone()).unwrap();
                let unmet: Vec<&str> = describe.unmet_runner_groups(&declared);
                let expected: Vec<&str> = case["unmet_runner_groups"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|g| g.as_str().unwrap())
                    .collect();
                assert_eq!(unmet, expected, "{name}: unmet runner groups");
            }
        }
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            let problems = check_describe(&case["answer"]).expect_err(name);
            let names: Vec<&str> = problems.iter().map(DescribeProblem::name).collect();
            assert!(
                names.contains(&case["problem"].as_str().unwrap()),
                "{name}: {names:?}"
            );
        }
    }

    #[test]
    fn builders_match_decoding() {
        let built = RoleDescribe::new(
            vec![Major::new(
                PROVIDES,
                REQUIRED_OPS.iter().map(|op| op.to_string()).collect(),
                "alpha",
            )],
            "1.0.0",
        )
        .with_runner_groups(vec!["transcript_reads".into()]);
        let encoded = serde_json::to_value(&built).unwrap();
        assert_eq!(check_describe(&encoded).unwrap(), built);
        assert_eq!(built.major(PROVIDES).unwrap().stability(), Stability::Alpha);
    }
}
