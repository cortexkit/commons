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

use crate::{
    check_capability_tag, ops, CapabilityTagProblem, CATALOG_CAPABILITY_FILTER, PROVIDES,
    REQUIRED_OPS,
};

/// The `role.describe` answer: every major of the role the module serves,
/// each with its own ops and stability. The caller picks the highest major it
/// understands for a new session.
///
/// Decoded leniently: unknown fields, majors, ops and capabilities are
/// ignored. Only the presence of the `tool-provider/v1` major and its required
/// ops are checked strictly, by [`check_describe`].
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct RoleDescribe {
    pub majors: Vec<Major>,
    /// The module's own build identity, for display. Never a cache key.
    pub implementation_version: String,
    /// Optional module-level capabilities. `tool-provider/v1` defines
    /// [`CATALOG_CAPABILITY_FILTER`]; session capabilities travel in the
    /// catalog answer instead.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// The provider's sorted, unique union of catalog tool capability tags,
    /// present only when it advertises [`CATALOG_CAPABILITY_FILTER`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_capabilities: Vec<String>,
}

impl RoleDescribe {
    /// A role description with no optional module capabilities.
    pub fn new(majors: Vec<Major>, implementation_version: impl Into<String>) -> Self {
        Self {
            majors,
            implementation_version: implementation_version.into(),
            capabilities: Vec::new(),
            tool_capabilities: Vec::new(),
        }
    }

    /// Set the module-level capabilities.
    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }

    /// Set the tool capability tags, sorted by UTF-8 bytes and deduplicated.
    pub fn with_tool_capabilities<I, S>(mut self, capabilities: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut capabilities: Vec<String> = capabilities.into_iter().map(Into::into).collect();
        capabilities.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        capabilities.dedup();
        self.tool_capabilities = capabilities;
        self
    }
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

/// Intersect a consumer's wanted tool tags with the provider's advertised
/// tags. The result is sorted and deduplicated for `params.capabilities`; a
/// provider that does not advertise [`CATALOG_CAPABILITY_FILTER`] returns
/// `None` so the consumer can retain its existing catalog request behavior.
///
/// Run [`check_describe`] first. A provider that advertises the filter but
/// lists no `tool_capabilities` (one built before this field existed) fails it
/// with [`DescribeProblem::MissingToolCapabilities`]; skip that provider's
/// tools rather than calling this, because an empty result here would mean "no
/// tag in common" and send a request that selects nothing.
pub fn select_tool_capabilities<I, S>(describe: &RoleDescribe, wanted: I) -> Option<Vec<String>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    if !describe
        .capabilities
        .iter()
        .any(|capability| capability == CATALOG_CAPABILITY_FILTER)
    {
        return None;
    }
    let wanted: std::collections::BTreeSet<String> = wanted
        .into_iter()
        .map(|tag| tag.as_ref().to_owned())
        .collect();
    let mut selected: Vec<String> = describe
        .tool_capabilities
        .iter()
        .filter(|tag| wanted.contains(*tag))
        .cloned()
        .collect();
    selected.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    selected.dedup();
    Some(selected)
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
    /// The provider advertises capability filtering but does not list its
    /// served tool capability tags.
    MissingToolCapabilities,
    /// Tool capability tags are not in bytewise sorted order.
    UnsortedToolCapabilities,
    /// A tool capability tag appears more than once.
    DuplicateToolCapabilities,
    /// A tool capability tag does not pass this role's tag grammar.
    InvalidToolCapabilityTag {
        tag: String,
        problem: CapabilityTagProblem,
    },
    /// The provider lists served tags without advertising capability filtering.
    ToolCapabilitiesWithoutFilter,
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
        let advertises_filter = describe
            .capabilities
            .iter()
            .any(|capability| capability == CATALOG_CAPABILITY_FILTER);
        if advertises_filter {
            if describe.tool_capabilities.is_empty() {
                problems.push(DescribeProblem::MissingToolCapabilities);
            } else {
                for tag in &describe.tool_capabilities {
                    if let Err(problem) = check_capability_tag(tag) {
                        problems.push(DescribeProblem::InvalidToolCapabilityTag {
                            tag: tag.clone(),
                            problem,
                        });
                    }
                }
                for pair in describe.tool_capabilities.windows(2) {
                    if pair[0].as_bytes() == pair[1].as_bytes() {
                        problems.push(DescribeProblem::DuplicateToolCapabilities);
                    } else if pair[0].as_bytes() > pair[1].as_bytes() {
                        problems.push(DescribeProblem::UnsortedToolCapabilities);
                    }
                }
            }
        } else if object.contains_key("tool_capabilities") {
            problems.push(DescribeProblem::ToolCapabilitiesWithoutFilter);
        }
        if problems.is_empty() {
            Ok(describe)
        } else {
            Err(problems)
        }
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
                    DescribeProblem::MissingToolCapabilities => "missing_tool_capabilities",
                    DescribeProblem::UnsortedToolCapabilities => "unsorted_tool_capabilities",
                    DescribeProblem::DuplicateToolCapabilities => "duplicate_tool_capabilities",
                    DescribeProblem::InvalidToolCapabilityTag { .. } => {
                        "invalid_tool_capability_tag"
                    }
                    DescribeProblem::ToolCapabilitiesWithoutFilter => {
                        "tool_capabilities_without_filter"
                    }
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

    #[test]
    fn role_describe_omits_empty_tool_capabilities_on_the_wire() {
        let describe = RoleDescribe::new(Vec::new(), "test")
            .with_capabilities(vec![CATALOG_CAPABILITY_FILTER.into()]);
        let value = serde_json::to_value(&describe).unwrap();
        assert!(!value.as_object().unwrap().contains_key("tool_capabilities"));
        let decoded: RoleDescribe = serde_json::from_value(value).unwrap();
        assert!(decoded.tool_capabilities.is_empty());
    }

    #[test]
    fn consumer_selects_the_sorted_intersection_only_for_advertising_providers() {
        let describe = RoleDescribe::new(
            vec![Major {
                version: PROVIDES.into(),
                ops: REQUIRED_OPS.iter().map(|op| (*op).to_owned()).collect(),
                stability: "alpha".into(),
            }],
            "test",
        )
        .with_capabilities(vec![CATALOG_CAPABILITY_FILTER.into()])
        .with_tool_capabilities(["zeta:tool/v1", "code.read/v1", "code.read/v1"]);
        assert_eq!(
            select_tool_capabilities(
                &describe,
                ["zeta:tool/v1", "code.read/v1", "missing:tool/v1"]
            ),
            Some(vec!["code.read/v1".into(), "zeta:tool/v1".into()])
        );
        let legacy = RoleDescribe::new(Vec::new(), "test");
        assert_eq!(select_tool_capabilities(&legacy, ["code.read/v1"]), None);
    }
}
