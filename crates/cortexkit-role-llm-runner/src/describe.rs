//! `role.describe`: discovery before use.
//!
//! A consumer that finds a module claiming `llm-runner/v1` calls
//! `role.describe`, refuses the module by name if a required op is missing,
//! and checks that every capability it needs is declared before using it.
//! The answer describes the runner build, never a session: session-level
//! capabilities come from the admission reply or `session.baseline`, and the
//! answer says which (`session_capabilities_from`). It depends only on the
//! module build, so a consumer may cache it for as long as it talks to the
//! same module incarnation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{capabilities, PROVIDES, REQUIRED_OPS};

/// The `role.describe` answer: every major of the role the module serves,
/// each with its own ops and stability, plus the module-level capabilities.
///
/// Decoded leniently: unknown fields, majors, ops and capabilities are
/// ignored. [`check_describe`] checks strictly only what a consumer relies
/// on: the `llm-runner/v1` major, its required ops, the ops of every group
/// the answer declares, and where session capabilities come from.
///
/// Non-exhaustive so a later optional member is additive: build one with
/// [`RoleDescribe::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RoleDescribe {
    pub majors: Vec<Major>,
    /// The module's own build identity, for display. Never a cache key.
    pub implementation_version: String,
    /// Module-level capabilities: the capability groups
    /// ([`capabilities::GROUPS`]) and `ordered_hook_phases` where it holds
    /// for every session.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Where session-level capabilities come from: `admission` or
    /// `baseline` ([`capabilities::source`]). Required by
    /// [`check_describe`]; optional in the type so an answer without it
    /// still decodes and is refused by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_capabilities_from: Option<String>,
}

/// One served major.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Major {
    /// The role and major as the manifest spells it, for example
    /// `llm-runner/v1`.
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
    /// An answer for the given majors and build identity, declaring no
    /// capability and no session-capability source.
    pub fn new(majors: Vec<Major>, implementation_version: impl Into<String>) -> Self {
        Self {
            majors,
            implementation_version: implementation_version.into(),
            ..Self::default()
        }
    }

    /// Set the module-level capabilities.
    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Say where session-level capabilities come from.
    pub fn with_session_capabilities_from(mut self, source: impl Into<String>) -> Self {
        self.session_capabilities_from = Some(source.into());
        self
    }

    /// The entry for `version` (for example `llm-runner/v1`), if served.
    pub fn major(&self, version: &str) -> Option<&Major> {
        self.majors.iter().find(|major| major.version == version)
    }

    /// Whether the module-level capability `name` is declared.
    pub fn declares(&self, name: &str) -> bool {
        self.capabilities.iter().any(|declared| declared == name)
    }

    /// The capabilities in `required` this answer does not declare, in the
    /// order given. A consumer refuses the module, naming them, when this is
    /// not empty.
    pub fn missing<'a>(&self, required: &[&'a str]) -> Vec<&'a str> {
        required
            .iter()
            .copied()
            .filter(|name| !self.declares(name))
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

/// Why a `role.describe` answer does not describe a usable `llm-runner/v1`
/// module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DescribeProblem {
    /// The answer is not a JSON object.
    NotAnObject,
    /// A required field is missing or has the wrong type.
    Undecodable(String),
    /// No entry in `majors` is `llm-runner/v1`.
    MissingMajor { found: Vec<String> },
    /// The `llm-runner/v1` entry lacks a required op.
    MissingOp(&'static str),
    /// A declared capability group lacks one of its ops. A group is all or
    /// nothing, so a consumer cannot use a partly served group.
    GroupIncomplete {
        capability: &'static str,
        op: &'static str,
    },
    /// The answer does not say where session-level capabilities come from.
    MissingSessionCapabilitiesSource,
}

/// Decode `raw` and check it lists an `llm-runner/v1` major with every
/// required op, every op of each declared group, and a session-capability
/// source. Every problem is returned, not only the first. Unknown
/// capabilities and an unknown source value are tolerated.
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
            for (group, group_ops) in capabilities::GROUPS {
                if !describe.declares(group) {
                    continue;
                }
                for op in *group_ops {
                    if !major.serves(op) {
                        problems.push(DescribeProblem::GroupIncomplete {
                            capability: group,
                            op,
                        });
                    }
                }
            }
        }
    }
    if describe.session_capabilities_from.is_none() {
        problems.push(DescribeProblem::MissingSessionCapabilitiesSource);
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
    use crate::{capabilities::source, ops, vectors};

    fn problem_name(problem: &DescribeProblem) -> &'static str {
        match problem {
            DescribeProblem::NotAnObject => "not_an_object",
            DescribeProblem::Undecodable(_) => "undecodable",
            DescribeProblem::MissingMajor { .. } => "missing_major",
            DescribeProblem::MissingOp(_) => "missing_op",
            DescribeProblem::GroupIncomplete { .. } => "group_incomplete",
            DescribeProblem::MissingSessionCapabilitiesSource => {
                "missing_session_capabilities_source"
            }
        }
    }

    #[test]
    fn describe_vectors_check_as_recorded() {
        let file = vectors::load("role-describe.json");
        for case in vectors::cases(&file, "accepted") {
            let name = case["name"].as_str().unwrap();
            let describe = check_describe(&case["answer"])
                .unwrap_or_else(|problems| panic!("{name}: {problems:?}"));
            for capability in case["declares"].as_array().unwrap() {
                assert!(describe.declares(capability.as_str().unwrap()), "{name}");
            }
        }
        for case in vectors::cases(&file, "canonical") {
            vectors::round_trip::<RoleDescribe>(case["name"].as_str().unwrap(), &case["answer"]);
        }
        for case in vectors::cases(&file, "refused") {
            let name = case["name"].as_str().unwrap();
            let problems = check_describe(&case["answer"]).expect_err(name);
            let names: Vec<&str> = problems.iter().map(problem_name).collect();
            let expected = case["problem"].as_str().unwrap();
            assert!(names.contains(&expected), "{name}: {names:?}");
        }
    }

    #[test]
    fn unknown_stability_extra_majors_ops_and_capabilities_are_tolerated() {
        let answer = serde_json::json!({
            "majors": [
                {"version": "llm-runner/v1", "stability": "experimental",
                 "ops": ["role.describe", "session.baseline", "compaction.ready", "vendor.extra"]},
                {"version": "llm-runner/v2", "stability": "alpha", "ops": ["role.describe"]}
            ],
            "implementation_version": "1.2.3",
            "capabilities": ["vendor_feature"],
            "session_capabilities_from": "somewhere_new",
            "future_field": {}
        });
        let describe = check_describe(&answer).unwrap();
        let v1 = describe.major(PROVIDES).unwrap();
        assert_eq!(v1.stability(), Stability::Other("experimental".into()));
        assert!(describe.declares("vendor_feature"));
        assert_eq!(
            describe.major("llm-runner/v2").unwrap().stability(),
            Stability::Alpha
        );
    }

    #[test]
    fn builders_produce_a_checkable_answer_and_name_missing_capabilities() {
        let describe = RoleDescribe::new(
            vec![Major::new(
                PROVIDES,
                vec![
                    ops::ROLE_DESCRIBE.into(),
                    ops::SESSION_BASELINE.into(),
                    ops::COMPACTION_READY.into(),
                    ops::SESSION_SEND.into(),
                ],
                "alpha",
            )],
            "0.0.1",
        )
        .with_capabilities(vec![capabilities::STEER.into()])
        .with_session_capabilities_from(source::BASELINE);
        let raw = serde_json::to_value(&describe).unwrap();
        assert_eq!(check_describe(&raw).unwrap(), describe);
        assert_eq!(
            describe.missing(&[capabilities::STEER, capabilities::TRANSCRIPT_READS]),
            vec![capabilities::TRANSCRIPT_READS]
        );
    }
}
