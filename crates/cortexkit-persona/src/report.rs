//! The resolution report of SPEC.md section 7: deterministic and serialisable,
//! so a consumer can show which tiers and fragments a render used and why
//! others were skipped.

use serde::{Deserialize, Serialize};

use crate::{ApprovalState, Digest, TrustClass};

/// One tier, in ladder order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TierReport {
    /// The tier's name.
    pub name: String,
    /// Its trust class.
    pub trust: TrustClass,
    /// The tier root as the module gave it; `null` for an embedded tier.
    pub root: Option<String>,
    /// The role's layer path relative to the root, `<roles_path>/<role>`.
    pub relative_path: String,
    /// Whether the tier has a `PERSONA.md` for the role.
    pub present: bool,
    /// Whether the layer may apply.
    pub approval: ApprovalState,
    /// The layer digest, when present.
    pub layer_digest: Option<Digest>,
    /// Whether the layer was applied.
    pub used: bool,
}

/// Another role's layer loaded as an explicit `extends` parent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ParentReport {
    /// The parent's role.
    pub role: String,
    /// The tier it was read from.
    pub tier: String,
    /// Its tier-relative path.
    pub relative_path: String,
    /// Whether it may apply.
    pub approval: ApprovalState,
    /// Its layer digest.
    pub layer_digest: Option<Digest>,
}

/// Where a fragment's text came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
#[non_exhaustive]
pub enum FragmentOrigin {
    /// A bundle file, from the most specific applied tier that has it.
    Bundle {
        /// The tier.
        tier: String,
    },
    /// A source the module supplied at render time; it carries the module's
    /// trust, whatever tier included it.
    Supplied,
}

/// One fragment of the composed list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FragmentReport {
    /// The fragment id.
    pub id: String,
    /// Where its text came from.
    pub origin: FragmentOrigin,
    /// SHA-256 of its LF-normalised text.
    pub digest: Digest,
    /// Whether it is in the entry template's include closure, and so counts
    /// toward the source revision.
    pub included: bool,
}

/// What the engine did for one render.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResolutionReport {
    /// The report format, `persona/v1`.
    pub format: String,
    /// The module.
    pub module: String,
    /// The role.
    pub role: String,
    /// Every tier, in ladder order.
    pub tiers: Vec<TierReport>,
    /// Other roles' layers used as explicit `extends` parents.
    pub parents: Vec<ParentReport>,
    /// The composed fragment list, in declared order.
    pub fragments: Vec<FragmentReport>,
    /// The tier whose body is the entry template.
    pub entry_tier: String,
    /// The source revision.
    pub source_revision: Digest,
    /// SHA-256 of the rendered bytes.
    pub rendered_digest: Digest,
}
