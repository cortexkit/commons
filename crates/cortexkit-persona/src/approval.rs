//! Project-tier approvals. The module stores them and records one only after
//! the operator confirms in person; the engine only reads the set it is handed.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{Digest, ProjectKey};

/// One approval: the operator accepted this exact layer of this role, for
/// this project, at this tier-relative path.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Approval {
    /// The module the approval belongs to.
    pub module: String,
    /// The role.
    pub role: String,
    /// The opaque project identity.
    pub project_key: ProjectKey,
    /// The layer's tier-relative path, `<roles_path>/<role>`.
    pub relative_path: String,
    /// The approved layer digest.
    pub layer_digest: Digest,
}

/// Whether a layer may apply, as listed in the resolution report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalState {
    /// No approval is needed: an `operator` layer that applies as found, or a
    /// tier that has no layer at all.
    NotRequired,
    /// A `repository` layer whose digest the operator approved.
    Approved,
    /// A `repository` layer with no approval for this project and path.
    Unapproved,
    /// A `repository` layer that was approved with a different digest: its
    /// bytes changed since the operator looked.
    Changed,
}

impl ApprovalState {
    /// Whether a present layer in this state applies.
    pub fn applies(self) -> bool {
        matches!(self, ApprovalState::NotRequired | ApprovalState::Approved)
    }
}

/// The approvals a module hands to [`crate::PersonaEngine::load`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ApprovalSet {
    approvals: BTreeSet<Approval>,
}

impl ApprovalSet {
    /// An empty set: no repository layer applies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an approval.
    pub fn insert(&mut self, approval: Approval) {
        self.approvals.insert(approval);
    }

    /// Iterate the approvals in order.
    pub fn iter(&self) -> impl Iterator<Item = &Approval> {
        self.approvals.iter()
    }

    /// The state of a present repository layer. Without a project key nothing
    /// can match, so the layer is unapproved.
    pub(crate) fn state(
        &self,
        module: &str,
        role: &str,
        project_key: Option<&ProjectKey>,
        relative_path: &str,
        digest: Digest,
    ) -> ApprovalState {
        let Some(project_key) = project_key else {
            return ApprovalState::Unapproved;
        };
        let mut same_place = false;
        for a in &self.approvals {
            if a.module == module
                && a.role == role
                && &a.project_key == project_key
                && a.relative_path == relative_path
            {
                if a.layer_digest == digest {
                    return ApprovalState::Approved;
                }
                same_place = true;
            }
        }
        if same_place {
            ApprovalState::Changed
        } else {
            ApprovalState::Unapproved
        }
    }
}

impl FromIterator<Approval> for ApprovalSet {
    fn from_iter<I: IntoIterator<Item = Approval>>(iter: I) -> Self {
        ApprovalSet {
            approvals: iter.into_iter().collect(),
        }
    }
}
