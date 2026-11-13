//! The `role.describe` answer.
//!
//! `role.describe` takes empty params (`{}`) and answers
//! `{role, version, ops, consumers, requirement_fields, detail_supported,
//! capabilities?}`.

use serde::{Deserialize, Serialize};

use crate::{select::REQUIREMENT_FIELDS, REQUIRED_OPS, ROLE, VERSION_NUMBER};

/// The `role.describe` answer.
/// Build with [`Self::new`] and [`Self::with_capability`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RoleDescribe {
    /// Always `"routing"`.
    pub role: String,
    /// Always `1` for this major.
    pub version: u32,
    /// The ops the provider serves; every one of [`REQUIRED_OPS`].
    pub ops: Vec<String>,
    /// Every consumer id the provider's configuration knows.
    pub consumers: Vec<ConsumerEntry>,
    /// Every requirement member the provider reads. A member not listed is
    /// refused `invalid_requirements`.
    pub requirement_fields: Vec<String>,
    /// Whether the provider answers a `detail: true` select with its
    /// explanation object.
    pub detail_supported: bool,
    /// The optional groups the provider serves ([`crate::capabilities`]).
    /// Left out when empty, so a provider serving none answers exactly as
    /// before groups existed. A name this crate does not define is kept.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

/// One configured consumer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsumerEntry {
    pub id: String,
    /// The consumer is answered only with routes priced per token, never
    /// with a route paid from subscription quota.
    pub priced_only: bool,
}

impl RoleDescribe {
    /// An answer naming this role and major, every required op, and the
    /// original required requirement fields. Add optional fields to
    /// `requirement_fields` only when the router reads them.
    pub fn new(consumers: Vec<ConsumerEntry>, detail_supported: bool) -> Self {
        Self {
            role: ROLE.to_owned(),
            version: VERSION_NUMBER,
            ops: REQUIRED_OPS.iter().map(|op| (*op).to_owned()).collect(),
            consumers,
            requirement_fields: REQUIREMENT_FIELDS
                .iter()
                .filter(|field| !crate::select::OPTIONAL_REQUIREMENT_FIELDS.contains(field))
                .map(|field| (*field).to_owned())
                .collect(),
            detail_supported,
            capabilities: Vec::new(),
        }
    }

    /// Declare the optional group `capability`; declaring it twice lists it
    /// once.
    pub fn with_capability(mut self, capability: impl Into<String>) -> Self {
        let capability = capability.into();
        if !self.serves(&capability) {
            self.capabilities.push(capability);
        }
        self
    }

    /// Whether the provider declares the optional group `capability`.
    pub fn serves(&self, capability: &str) -> bool {
        self.capabilities.iter().any(|listed| listed == capability)
    }

    /// The consumer entry for `id`.
    pub fn consumer(&self, id: &str) -> Option<&ConsumerEntry> {
        self.consumers.iter().find(|entry| entry.id == id)
    }
}

/// Why a `role.describe` answer does not describe a `routing/v1` provider.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescribeProblem {
    WrongRole(String),
    WrongVersion(u32),
    MissingOp(&'static str),
    MissingRequirementField(&'static str),
    DuplicateConsumer(String),
    /// An optional operation and its capability group were not declared together.
    OpGroupMismatch {
        group: &'static str,
    },
    /// A capability group was declared without its prerequisite.
    MissingCapabilityDependency {
        group: &'static str,
        requires: &'static str,
    },
}

impl std::fmt::Display for DescribeProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::WrongRole(role) => write!(f, "role is {role:?}, not {ROLE:?}"),
            Self::WrongVersion(version) => {
                write!(f, "version is {version}, not {VERSION_NUMBER}")
            }
            Self::MissingOp(op) => write!(f, "ops does not list {op}"),
            Self::MissingRequirementField(field) => {
                write!(f, "requirement_fields does not list {field}")
            }
            Self::DuplicateConsumer(id) => write!(f, "consumer {id:?} is listed twice"),
            Self::OpGroupMismatch { group } => {
                write!(
                    f,
                    "operation and capability group {group:?} must be declared together"
                )
            }
            Self::MissingCapabilityDependency { group, requires } => {
                write!(f, "capability group {group:?} requires {requires:?}")
            }
        }
    }
}

/// Check the parts of a `role.describe` answer a consumer relies on before
/// routing anything to the provider.
pub fn check_describe(describe: &RoleDescribe) -> Result<(), DescribeProblem> {
    if describe.role != ROLE {
        return Err(DescribeProblem::WrongRole(describe.role.clone()));
    }
    if describe.version != VERSION_NUMBER {
        return Err(DescribeProblem::WrongVersion(describe.version));
    }
    for op in REQUIRED_OPS {
        if !describe.ops.iter().any(|listed| listed == op) {
            return Err(DescribeProblem::MissingOp(op));
        }
    }
    for field in REQUIREMENT_FIELDS {
        if crate::select::OPTIONAL_REQUIREMENT_FIELDS.contains(field) {
            continue;
        }
        if !describe
            .requirement_fields
            .iter()
            .any(|listed| listed == field)
        {
            return Err(DescribeProblem::MissingRequirementField(field));
        }
    }
    for &(op, group) in crate::capabilities::OP_GROUPS {
        if describe.ops.iter().any(|listed| listed == op) != describe.serves(group) {
            return Err(DescribeProblem::OpGroupMismatch { group });
        }
    }
    for (group, requires) in [
        (
            crate::capabilities::OBSERVE_CREDENTIAL_REFUSED,
            crate::capabilities::OBSERVE_BACKOFF,
        ),
        (
            crate::capabilities::ACCOUNT_CONTINUE_SIGNED_OUT,
            crate::capabilities::ACCOUNT_CONTINUE,
        ),
    ] {
        if describe.serves(group) && !describe.serves(requires) {
            return Err(DescribeProblem::MissingCapabilityDependency { group, requires });
        }
    }
    for group in [
        crate::capabilities::ENGINE_REPORTS,
        crate::capabilities::ENGINE_EVALS,
    ] {
        if describe.serves(group) && !describe.serves(crate::capabilities::SELECT_ENGINE) {
            return Err(DescribeProblem::MissingCapabilityDependency {
                group,
                requires: crate::capabilities::SELECT_ENGINE,
            });
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in &describe.consumers {
        if !seen.insert(entry.id.as_str()) {
            return Err(DescribeProblem::DuplicateConsumer(entry.id.clone()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn decision_input_fields_are_individually_opt_in() {
        let mut describe = super::RoleDescribe::new(vec![], false);
        assert_eq!(
            describe.requirement_fields,
            [
                "iq",
                "eq",
                "context_window",
                "max_output_tokens",
                "tools",
                "tool_capabilities",
                "modalities",
                "structured_output"
            ]
        );
        assert_eq!(super::check_describe(&describe), Ok(()));
        for field in ["questions", "input_tokens", "max_cost_usd"] {
            describe.requirement_fields.push(field.into());
            assert_eq!(super::check_describe(&describe), Ok(()));
        }
    }
    use super::*;

    #[test]
    fn optional_operations_require_exactly_their_group() {
        for &(op, group) in crate::capabilities::OP_GROUPS {
            let mut describe = RoleDescribe::new(vec![], false);
            describe.ops.push(op.into());
            assert_eq!(
                check_describe(&describe),
                Err(DescribeProblem::OpGroupMismatch { group })
            );
            describe.capabilities.push(group.into());
            if [
                crate::capabilities::ENGINE_REPORTS,
                crate::capabilities::ENGINE_EVALS,
            ]
            .contains(&group)
            {
                describe
                    .capabilities
                    .push(crate::capabilities::SELECT_ENGINE.into());
                describe.ops.push(crate::ops::ROUTE_SELECT_ENGINE.into());
            }
            assert_eq!(check_describe(&describe), Ok(()));
            describe.ops.retain(|listed| listed != op);
            assert_eq!(
                check_describe(&describe),
                Err(DescribeProblem::OpGroupMismatch { group })
            );
        }
    }

    #[test]
    fn signed_out_continuation_requires_continue_by_name() {
        use crate::capabilities::{ACCOUNT_CONTINUE, ACCOUNT_CONTINUE_SIGNED_OUT};
        let describe =
            RoleDescribe::new(vec![], false).with_capability(ACCOUNT_CONTINUE_SIGNED_OUT);
        assert_eq!(
            check_describe(&describe),
            Err(DescribeProblem::MissingCapabilityDependency {
                group: ACCOUNT_CONTINUE_SIGNED_OUT,
                requires: ACCOUNT_CONTINUE,
            })
        );
        assert_eq!(
            check_describe(&describe.with_capability(ACCOUNT_CONTINUE)),
            Ok(())
        );
    }

    #[test]
    fn observe_credential_refused_requires_backoff_by_name() {
        use crate::capabilities::{OBSERVE_BACKOFF, OBSERVE_CREDENTIAL_REFUSED};
        let describe = RoleDescribe::new(vec![], false).with_capability(OBSERVE_CREDENTIAL_REFUSED);
        let problem = DescribeProblem::MissingCapabilityDependency {
            group: OBSERVE_CREDENTIAL_REFUSED,
            requires: OBSERVE_BACKOFF,
        };
        assert_eq!(check_describe(&describe), Err(problem.clone()));
        assert_eq!(
            problem.to_string(),
            "capability group \"observe_credential_refused\" requires \"observe_backoff\""
        );
        assert_eq!(
            check_describe(&describe.with_capability(OBSERVE_BACKOFF)),
            Ok(())
        );
        assert_eq!(
            check_describe(&RoleDescribe::new(vec![], false).with_capability(OBSERVE_BACKOFF)),
            Ok(())
        );
    }

    #[test]
    fn legacy_060_describe_passes_and_round_trips_byte_for_byte() {
        let wire = r#"{"role":"routing","version":1,"ops":["role.describe","route.select","route.report","route.account_for","route.account_observe"],"consumers":[],"requirement_fields":["iq","eq","context_window","max_output_tokens","tools","tool_capabilities","modalities","structured_output"],"detail_supported":false}"#;
        let describe: RoleDescribe = serde_json::from_str(wire).unwrap();
        assert_eq!(check_describe(&describe), Ok(()));
        assert_eq!(serde_json::to_string(&describe).unwrap(), wire);
    }

    #[test]
    fn new_passes_its_own_check() {
        let describe = RoleDescribe::new(
            vec![ConsumerEntry {
                id: "codemode".into(),
                priced_only: true,
            }],
            false,
        );
        assert_eq!(check_describe(&describe), Ok(()));
        assert!(describe.consumer("codemode").unwrap().priced_only);
    }

    #[test]
    fn a_missing_op_or_field_is_named() {
        let mut describe = RoleDescribe::new(vec![], false);
        describe.ops.retain(|op| op != "route.report");
        assert_eq!(
            check_describe(&describe),
            Err(DescribeProblem::MissingOp("route.report"))
        );
        let mut describe = RoleDescribe::new(vec![], false);
        describe
            .requirement_fields
            .retain(|f| f != "tool_capabilities");
        assert_eq!(
            check_describe(&describe),
            Err(DescribeProblem::MissingRequirementField(
                "tool_capabilities"
            ))
        );
    }
}
