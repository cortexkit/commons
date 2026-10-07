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

/// Steer delivery receipt policy as declared in `role.describe`.
pub mod steer_receipt {
    /// On a guaranteed runner, a durably accepted steer is delivered:
    /// `delivered` may be absent, and must never be `pending` or `unknown`.
    /// The default when `steer_receipt` is absent.
    pub const GUARANTEED: &str = "guaranteed";
    /// On a confirm runner, an absent `delivered` means `pending`, never
    /// delivered.
    pub const CONFIRM: &str = "confirm";
}

/// The `role.describe` answer: every major of the role the module serves,
/// each with its own ops and stability, plus the module-level capabilities.
///
/// Decoded leniently: unknown fields, majors, ops and capabilities are
/// ignored. [`check_describe`] checks strictly only what a consumer relies
/// on: the `llm-runner/v1` major, its required ops, the ops of every group
/// the answer declares, where session capabilities come from, and the
/// read byte cap when the answer declares `transcript_reads`.
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
    /// The `session.read` byte cap: the default a page stops at when the
    /// request names no `max_bytes`, and the largest `max_bytes` the runner
    /// honours. Required by [`check_describe`] when the answer declares
    /// `transcript_reads`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_bytes: Option<MaxBytes>,
    /// Steer delivery receipt policy: `guaranteed` or `confirm`
    /// ([`steer_receipt`]). Absent means [`steer_receipt::GUARANTEED`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steer_receipt: Option<String>,
    /// Limits required when the `retention` capability group is declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<Retention>,
}

/// Whole-session retention limits advertised by a runner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Retention {
    /// Largest accepted `session.send.retention`, in seconds; must be positive.
    pub max_seconds: u64,
    /// Maximum delay from expiry to deletion of all runner-held content.
    pub delete_within_ms: u64,
}

impl Retention {
    pub fn new(max_seconds: u64, delete_within_ms: u64) -> Self {
        Self {
            max_seconds,
            delete_within_ms,
        }
    }
}

/// A runner's `session.read` byte cap, in bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MaxBytes {
    /// The cap a page stops at when the request names no `max_bytes`.
    pub default: u64,
    /// The largest cap the runner honours; a larger request is capped to it.
    pub maximum: u64,
}

fn alpha() -> String {
    "alpha".to_owned()
}

/// One served major.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Major {
    /// The role and major as the manifest spells it, for example
    /// `llm-runner/v1`.
    pub version: String,
    pub ops: Vec<String>,
    /// `alpha`, `beta` or `stable`; see [`Major::stability`]. Stability is
    /// data, never part of a capability name. An answer that omits it is
    /// read as `alpha`, the level every major starts at.
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

    /// State the `session.read` byte cap.
    pub fn with_max_bytes(mut self, default: u64, maximum: u64) -> Self {
        self.max_bytes = Some(MaxBytes { default, maximum });
        self
    }

    /// State the retention limits (the capability must also be listed).
    pub fn with_retention(mut self, max_seconds: u64, delete_within_ms: u64) -> Self {
        self.retention = Some(Retention::new(max_seconds, delete_within_ms));
        self
    }

    /// Say where session-level capabilities come from.
    pub fn with_session_capabilities_from(mut self, source: impl Into<String>) -> Self {
        self.session_capabilities_from = Some(source.into());
        self
    }

    /// Say what steer delivery receipt policy the runner follows.
    pub fn with_steer_receipt(mut self, steer_receipt: impl Into<String>) -> Self {
        self.steer_receipt = Some(steer_receipt.into());
        self
    }

    /// The steer delivery receipt policy, with absence read as [`steer_receipt::GUARANTEED`].
    pub fn steer_receipt(&self) -> &str {
        self.steer_receipt
            .as_deref()
            .unwrap_or(steer_receipt::GUARANTEED)
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
    MissingMajor {
        found: Vec<String>,
    },
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
    /// The answer declares `transcript_reads` without stating its byte cap.
    MissingMaxBytes,
    /// The retention group has no limits, or cannot accept a positive value.
    MissingRetention,
    InvalidRetention,
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
    if describe.declares(capabilities::TRANSCRIPT_READS) && describe.max_bytes.is_none() {
        problems.push(DescribeProblem::MissingMaxBytes);
    }
    if describe.declares(capabilities::RETENTION) {
        match describe.retention {
            None => problems.push(DescribeProblem::MissingRetention),
            Some(limits) if limits.max_seconds == 0 => {
                problems.push(DescribeProblem::InvalidRetention)
            }
            Some(_) => {}
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
            DescribeProblem::MissingMaxBytes => "missing_max_bytes",
            DescribeProblem::MissingRetention => "missing_retention",
            DescribeProblem::InvalidRetention => "invalid_retention",
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
        for case in vectors::cases(&file, "stability_defaults") {
            let name = case["name"].as_str().unwrap();
            let describe = check_describe(&case["answer"])
                .unwrap_or_else(|problems| panic!("{name}: {problems:?}"));
            let major = describe.major(PROVIDES).unwrap();
            assert_eq!(
                major.stability,
                case["stability"].as_str().unwrap(),
                "{name}"
            );
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

    #[test]
    fn steer_receipt_vectors_round_trip_and_defaults() {
        let file = vectors::load("role-describe.json");
        let mut seen_guaranteed = false;
        let mut seen_confirm = false;
        let mut seen_absent = false;
        for case in vectors::cases(&file, "canonical") {
            let name = case["name"].as_str().unwrap();
            let describe: RoleDescribe = vectors::round_trip(name, &case["answer"]);
            match describe.steer_receipt.as_deref() {
                Some("guaranteed") => seen_guaranteed = true,
                Some("confirm") => seen_confirm = true,
                None => seen_absent = true,
                _ => {}
            }
        }
        assert!(seen_guaranteed);
        assert!(seen_confirm);
        assert!(seen_absent);

        let default_describe = RoleDescribe::new(vec![], "1.0");
        assert_eq!(default_describe.steer_receipt, None);
        assert_eq!(default_describe.steer_receipt(), steer_receipt::GUARANTEED);

        let confirm_describe = default_describe
            .clone()
            .with_steer_receipt(steer_receipt::CONFIRM);
        assert_eq!(
            confirm_describe.steer_receipt.as_deref(),
            Some(steer_receipt::CONFIRM)
        );
        assert_eq!(confirm_describe.steer_receipt(), steer_receipt::CONFIRM);
    }
}

#[cfg(test)]
mod retention_tests {
    use super::{check_describe, DescribeProblem, Major, Retention, RoleDescribe};
    use crate::{capabilities, ops, PROVIDES, REQUIRED_OPS};

    #[test]
    fn retention_group_requires_positive_limits() {
        let mut ops: Vec<String> = REQUIRED_OPS.iter().map(|s| (*s).into()).collect();
        ops.push(ops::SESSION_SEND.into());
        let base = RoleDescribe::new(vec![Major::new(PROVIDES, ops, "alpha")], "1")
            .with_capabilities(vec![capabilities::RETENTION.into()])
            .with_session_capabilities_from("baseline");
        let check = |value: &RoleDescribe| check_describe(&serde_json::to_value(value).unwrap());
        assert_eq!(check(&base), Err(vec![DescribeProblem::MissingRetention]));
        assert_eq!(
            check(&base.clone().with_retention(0, 0)),
            Err(vec![DescribeProblem::InvalidRetention])
        );
        let described = check(&base.with_retention(u64::MAX, 0)).unwrap();
        assert_eq!(described.retention, Some(Retention::new(u64::MAX, 0)));
        let decoded: Retention =
            serde_json::from_str(r#"{"max_seconds":3,"delete_within_ms":100,"future":true}"#)
                .unwrap();
        assert_eq!(decoded, Retention::new(3, 100));
    }
}
