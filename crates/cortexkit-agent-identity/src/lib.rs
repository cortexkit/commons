//! Serde-only agent, session, fleet and context reads for the agent registry.
//!
//! No I/O, path normalization, admission checks, or cross-field validation.

#![doc = include_str!("../README.md")]

pub use cortexkit_project_identity::{
    Outcome, Presence, Refusal, RefusalCode, Request, Success, Via,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

macro_rules! open_string {
    ($(#[$attr:meta])* $name:ident { $($variant:ident => $wire:literal),+ $(,)? }) => {
        $(#[$attr])*
        #[non_exhaustive]
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(from = "String", into = "String")]
        pub enum $name {
            $($variant,)+
            /// An unfamiliar spelling, preserved without interpretation.
            Other(String),
        }
        impl $name {
            /// Recognizes current spellings and preserves all other strings.
            pub fn new(value: impl Into<String>) -> Self { Self::from(value.into()) }
        }
        impl From<String> for $name {
            fn from(value: String) -> Self {
                match value.as_str() {
                    $($wire => Self::$variant,)+
                    _ => Self::Other(value),
                }
            }
        }
        impl From<$name> for String {
            fn from(value: $name) -> Self {
                match value {
                    $($name::$variant => $wire.to_owned(),)+
                    $name::Other(value) => value,
                }
            }
        }
    };
}

macro_rules! wire_struct {
    ($(#[$doc:meta])* $name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        // Caller attributes come after the derive so serde container
        // attributes such as `deny_unknown_fields` are accepted.
        #[non_exhaustive]
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        $(#[$doc])*
        pub struct $name { $($(#[$attr])* pub $field: $ty,)* }
        impl $name {
            /// Constructs data without checking cross-field writer invariants.
            #[allow(clippy::too_many_arguments)]
            pub fn new($($field: $ty),*) -> Self { Self { $($field,)* } }
        }
    };
}

wire_struct!(
    /// Params refuse unknown members, as the agent registry does.
    #[serde(deny_unknown_fields)]
    ContextProjectParams { folder: String }
);
wire_struct!(
    /// Shared single-agent params; the wire spelling is always `agent_id`.
    /// Unknown members are refused, as the agent registry does.
    #[serde(deny_unknown_fields)]
    AgentIdParams { agent_id: String }
);
pub type ContextSessionParams = AgentIdParams;
pub type AgentResolveParams = AgentIdParams;
pub type AgentWhereParams = AgentIdParams;
wire_struct!(
    /// Unknown members are refused, as the agent registry does.
    #[serde(deny_unknown_fields)]
    AgentWhereManyParams {
        #[serde(rename = "agentIds")]
        agent_ids: Vec<String>,
    }
);

wire_struct!(
    /// One read transaction's local journal and shared-log positions.
    ContextStamp {
        incarnation: String,
        generation: i64,
        #[serde(rename = "logPosition")]
        log_position: i64,
    }
);
wire_struct!(
    /// Accepted workspace document metadata, not its charter or document text.
    ContextDocument {
        revision: i64, sha256: String, charter_bytes: i64, document_bytes: i64,
    }
);
wire_struct!(ContextProjectReply {
    project_id: Option<String>, workspace_id: Option<String>, domain: Option<String>,
    via: Via, document: Option<ContextDocument>, members_digest: Option<String>, stamp: ContextStamp,
});

open_string!(AgentStatus {
    Unknown => "unknown", Live => "live", Retired => "retired", Merged => "merged",
});
open_string!(GoneReason { Deleted => "deleted", Merged => "merged" });
wire_struct!(AgentGone {
    reason: GoneReason,
    at: i64,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    into_agent_id: Presence<String>,
});
wire_struct!(
    /// Resolve data without the standalone operation's appended incarnation.
    ///
    /// In a successful `context.session`, status is live and both `gone` and
    /// `merged_into` are null: whereabouts refuses terminal identities first.
    CoreAgentResolution {
        agent_id: String, status: AgentStatus, merged_into: Option<String>,
        gone: Option<AgentGone>, generation: i64,
    }
);
wire_struct!(
    /// Narrow resolution without names, roles, tags, placement, or supervisor.
    /// Unknown and retired identities are successful answers, not refusals.
    AgentResolveReply {
        #[serde(flatten)]
        agent: CoreAgentResolution,
        incarnation: String,
    }
);

open_string!(
    /// Session-selected states only; unknown states do not grant activation permission.
    CatalogGenerationState { Ready => "ready", Active => "active", Degraded => "degraded" }
);
wire_struct!(CatalogGeneration {
    generation_id: String,
    state: CatalogGenerationState
});
open_string!(PinKind { MemoryProfile => "memory-profile", Persona => "persona" });

/// The read projects stored JSON without revalidating its state type.
/// `blocked`, not the state spelling, is the handler's evaluated decision.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "Value", into = "Value")]
pub enum PinStatusState {
    Available,
    Withdrawn,
    Tombstoned,
    Revoked,
    /// An unfamiliar string state, preserved without interpretation.
    Other(String),
    /// A non-string state projected from syntactically valid stored JSON.
    Json(Value),
}

impl PinStatusState {
    pub fn new(value: Value) -> Self {
        Self::from(value)
    }
}

impl From<Value> for PinStatusState {
    fn from(value: Value) -> Self {
        match value {
            Value::String(value) => match value.as_str() {
                "available" => Self::Available,
                "withdrawn" => Self::Withdrawn,
                "tombstoned" => Self::Tombstoned,
                "revoked" => Self::Revoked,
                _ => Self::Other(value),
            },
            value => Self::Json(value),
        }
    }
}

impl From<PinStatusState> for Value {
    fn from(value: PinStatusState) -> Self {
        match value {
            PinStatusState::Available => "available".into(),
            PinStatusState::Withdrawn => "withdrawn".into(),
            PinStatusState::Tombstoned => "tombstoned".into(),
            PinStatusState::Revoked => "revoked".into(),
            PinStatusState::Other(value) => value.into(),
            PinStatusState::Json(value) => value,
        }
    }
}

wire_struct!(PinStatus {
    state: PinStatusState,
    blocked: bool
});
wire_struct!(ContextPin {
    name: String, version: String, kind: PinKind, manifest_sha256: String,
    archive_sha256: String, publisher_fingerprint: String, status: Option<PinStatus>,
});
wire_struct!(ContextSessionReply {
    agent: CoreAgentResolution, catalog_generations: Vec<CatalogGeneration>,
    pins: Vec<ContextPin>,
    #[serde(rename = "where")]
    whereabouts: AgentWhere,
    stamp: ContextStamp,
});

open_string!(WhereStatus {
    Unclaimed => "unclaimed", HeldHere => "held_here", HeldElsewhere => "held_elsewhere",
});
open_string!(MoveSide { Source => "source", Receiver => "receiver" });
open_string!(MoveStep {
    Preparing => "preparing", Exported => "exported", Sealed => "sealed", Published => "published",
    Releasing => "releasing", PickedUp => "picked_up", Imported => "imported", Staged => "staged",
    Claimed => "claimed", Activating => "activating", Activated => "activated",
});
open_string!(MoveReason { Diverged => "diverged" });
open_string!(MoveStatus {
    WaitingForTarget => "waiting_for_target", AbortStuck => "abort_stuck",
    HeldAsFork => "held_as_fork", ContractViolation => "contract_violation",
});
open_string!(MoveErrorCode { CutAcceptPending => "cut_accept_pending" });
open_string!(MoveWarningCode { ReleaseNotEnforced => "release_not_enforced" });

wire_struct!(AgentMoving {
    cut_id: String, side: MoveSide, step: MoveStep, peer: String, peer_name: Option<String>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    reason: Presence<MoveReason>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    detail: Presence<Value>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    hold: Presence<Value>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    status: Presence<MoveStatus>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    message: Presence<String>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    waiting_since_ms: Presence<i64>,
    /// Added by direct whereabouts reads only, never by `context.session`.
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    waiting_ms: Presence<i64>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    diagnostics: Presence<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    diagnostic: Presence<Value>,
});
wire_struct!(MoveError {
    cut_id: String,
    code: MoveErrorCode
});
wire_struct!(MoveWarning {
    cut_id: String,
    module_id: String,
    code: MoveWarningCode,
    message: String,
});
wire_struct!(
    /// The handler adds `cut_id`; participant-defined payload members stay open JSON.
    MoveDiagnostic {
        cut_id: String,
        #[serde(flatten)]
        payload: Map<String, Value>,
    }
);
wire_struct!(
    /// Core whereabouts, also reused by session and batch reads.
    AgentWhere {
        known: bool, status: WhereStatus, holder: Option<String>, holder_name: Option<String>,
        epoch: i64, cut_id: Option<String>, read_at_ms: Option<i64>, stale: bool,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        moving: Presence<AgentMoving>,
        #[serde(rename = "moveErrors", default, skip_serializing_if = "Presence::is_missing")]
        move_errors: Presence<Vec<MoveError>>,
        #[serde(rename = "moveWarnings", default, skip_serializing_if = "Presence::is_missing")]
        move_warnings: Presence<Vec<MoveWarning>>,
        #[serde(rename = "moveDiagnostics", default, skip_serializing_if = "Presence::is_missing")]
        move_diagnostics: Presence<Vec<MoveDiagnostic>>,
    }
);
pub type AgentWhereReply = AgentWhere;
wire_struct!(AgentWhereEntry {
    agent_id: String,
    #[serde(flatten)]
    whereabouts: AgentWhere,
});
wire_struct!(AgentWhereManyReply { agents: Vec<AgentWhereEntry> });

wire_struct!(
    /// Exact reference lookup; bounds are checked by the agent registry only.
    #[serde(deny_unknown_fields)]
    AgentSessionForRefParams {
        ref_kind: String,
        #[serde(rename = "ref")]
        session_ref: String,
    }
);
wire_struct!(
    /// The wire member is `session`, with no `session_id` alias.
    #[serde(deny_unknown_fields)]
    AgentSessionForRunnerParams { runner: String, session: String }
);
wire_struct!(
    /// Reference lookup row, distinct from the runner lookup's lifecycle row.
    AgentSessionForRefReply {
        agent_id: String, ref_kind: String,
        #[serde(rename = "ref")]
        session_ref: String,
        harness: Option<String>, session_id: Option<String>, project_root: Option<String>,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        runner: Presence<String>,
        #[serde(rename = "heldOn", default, skip_serializing_if = "Presence::is_missing")]
        held_on: Presence<String>,
        incarnation: String,
    }
);
open_string!(SessionNetwork { Yes => "yes", No => "no", Unknown => "unknown" });
wire_struct!(SessionNetworkEntry {
    network: SessionNetwork,
    since_ms: i64
});
wire_struct!(
    /// Runner lookup row; a found row always has a non-null runner and session.
    AgentSessionForRunnerReply {
        agent_id: String, ref_kind: String,
        #[serde(rename = "ref")]
        session_ref: String,
        harness: Option<String>, session: String, project_root: Option<String>, runner: String,
        #[serde(rename = "heldOn", default, skip_serializing_if = "Presence::is_missing")]
        held_on: Presence<String>,
        ended_at_ms: Option<i64>, network: SessionNetwork,
        network_history: Vec<SessionNetworkEntry>, incarnation: String,
    }
);

wire_struct!(
    /// Every token value is accepted; only an exact current string is a cache hit.
    #[serde(deny_unknown_fields)]
    AgentFleetIdentityParams {
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        token: Presence<Value>,
    }
);
open_string!(FleetAgentRole {
    Assistant => "assistant", WorkspaceHead => "workspace_head", Head => "head", Hiree => "hiree",
});
open_string!(FleetAvatarType { Classic => "creature.classic", Wild => "creature.wild" });
wire_struct!(
    /// Avatar summary, never the genome itself.
    FleetAvatar {
        fingerprint: String,
        #[serde(rename = "type")]
        avatar_type: FleetAvatarType,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        version: Presence<i64>,
    }
);
wire_struct!(
    /// Fleet placement summary, not a full project record.
    FleetProject {
        id: String, name: String,
        #[serde(rename = "canonicalRoot", default, skip_serializing_if = "Presence::is_missing")]
        canonical_root: Presence<String>,
    }
);
wire_struct!(
    /// Fleet placement summary, not a full workspace record.
    FleetWorkspace { id: String, name: String }
);
wire_struct!(FleetAgentIdentity {
    #[serde(rename = "agentId")]
    agent_id: String,
    #[serde(rename = "displayName")]
    display_name: String,
    tag: String, labels: Vec<String>, role: FleetAgentRole,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    avatar: Presence<FleetAvatar>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    project: Presence<FleetProject>,
    #[serde(default, skip_serializing_if = "Presence::is_missing")]
    workspace: Presence<FleetWorkspace>,
});
wire_struct!(
    /// Full replies have `agents` (possibly empty); unchanged replies omit it.
    /// The reader preserves this distinction without validating the boolean invariant.
    AgentFleetIdentityReply {
        generation: i64, incarnation: String,
        #[serde(rename = "logPosition")]
        log_position: i64,
        token: String, unchanged: bool,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        agents: Presence<Vec<FleetAgentIdentity>>,
    }
);
