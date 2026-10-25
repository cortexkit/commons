//! `cortexkit-persona`: the shared engine a module uses to load, merge and
//! render the system text ("persona") of its own roles, as specified in
//! `SPEC.md` (persona/v1).
//!
//! A module builds a [`PersonaEngine`] for its module id, registers the
//! top-level keys it adds to `PERSONA.md` ([`Extension`]), and loads a role
//! from its tier ladder ([`Tier`], or [`DefaultLadder`] for the standard one)
//! with the approvals it stores for repository tiers ([`ApprovalSet`]). The
//! [`LoadedRole`] renders against a typed [`RenderContext`] and the qualified
//! fragments the module supplies ([`FragmentSources`]), returning the bytes,
//! their digest and a [`ResolutionReport`].
//!
//! ```
//! use cortexkit_persona::*;
//!
//! let base = EmbeddedTier::new().with_file(
//!     "greeter",
//!     "PERSONA.md",
//!     "---\nname: greeter\nversion: 1\ndisplay_name: Greeter\n---\nHello from {{ model.id }}.\n",
//! )?;
//! let tiers = [Tier::embedded("base", TrustClass::Operator, base)?];
//! let engine = PersonaEngine::new("demo")?;
//! let approvals = ApprovalSet::new();
//! let role = engine.load(LoadRequest::new("greeter", &tiers, &approvals))?;
//!
//! let context = RenderContext {
//!     agent: AgentContext { name: None, agent_id: None, project: None, workspace: None, role: "greeter".into() },
//!     model: ModelContext { provider: "p".into(), id: "m-1".into(), family: None, variant: "default".into() },
//!     harness: HarnessContext { id: "h".into(), capabilities: Default::default() },
//!     tools: GrantMap::default(),
//!     mcp: GrantMap::default(),
//!     modules: GrantMap::default(),
//!     demand: Demand { iq: 50, eq: 50 },
//!     env: EnvContext { os: OsName::Linux, timezone: "UTC".into(), locale: "en".into() },
//!     module: (),
//!     roots: Default::default(),
//!     redeclared: Default::default(),
//! };
//! let out = role.render(&context, &FragmentSources::new())?;
//! assert_eq!(out.text, "Hello from m-1.");
//! assert_eq!(out.report.entry_tier, "base");
//! # Ok::<(), PersonaError>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod approval;
mod context;
mod digest;
mod duplicates;
mod error;
mod extension;
mod ids;
mod load;
mod manifest;
mod render;
mod report;
mod template;
mod tier;
mod value;

pub use approval::{Approval, ApprovalSet, ApprovalState};
pub use context::{
    AgentContext, Demand, EnvContext, GrantMap, HarnessContext, ModelContext, OsName, RenderContext,
};
pub use digest::{normalize_newlines, Digest, DigestParseError};
pub use error::{PersonaError, RenderFailure, SourceRef};
pub use extension::{Extension, LayerExtensions, MergeFn, MergeInput, MergeRule};
pub use ids::{FragmentId, ProjectKey};
pub use load::{LoadRequest, LoadedRole, PersonaEngine};
pub use manifest::Manifest;
pub use render::{FragmentSources, Rendered, FORMAT};
pub use report::{FragmentOrigin, FragmentReport, ParentReport, ResolutionReport, TierReport};
pub use template::{FUEL_LIMIT, RECURSION_LIMIT};
pub use tier::{config_home, DefaultLadder, EmbeddedTier, Tier, TrustClass, MAX_FILE_BYTES};
pub use value::{TextValue, Value};
