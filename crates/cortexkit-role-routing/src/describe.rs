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
    /// An answer naming this role and major, every required op, and every
    /// requirement field this version defines.
    pub fn new(consumers: Vec<ConsumerEntry>, detail_supported: bool) -> Self {
        Self {
            role: ROLE.to_owned(),
            version: VERSION_NUMBER,
            ops: REQUIRED_OPS.iter().map(|op| (*op).to_owned()).collect(),
            consumers,
            requirement_fields: REQUIREMENT_FIELDS.iter().map(|f| (*f).to_owned()).collect(),
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
        if !describe
            .requirement_fields
            .iter()
            .any(|listed| listed == field)
        {
            return Err(DescribeProblem::MissingRequirementField(field));
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
    use super::*;

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
