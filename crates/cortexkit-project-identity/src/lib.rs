//! Serde-only request and reply shapes for the project-identity module.
//!
//! These types do not canonicalize paths, inspect Git, validate registrations,
//! or send requests. See the crate README for field presence and reader policy.

#![doc = include_str!("../README.md")]

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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

open_string!(
    /// Resolution provenance; path and durable-id reads use different subsets.
    Via {
        Root => "root", Walk => "walk", Containment => "containment",
        Worktree => "worktree", Implicit => "implicit", Current => "current",
        Alias => "alias", Gone => "gone",
    }
);
open_string!(
    /// Status of a captured checkout binding, not of the module process.
    BindingStatus {
        Active => "active", Unknown => "unknown", Replaced => "replaced", Retired => "retired",
    }
);
open_string!(
    /// Approval is separate from root identity.
    ApprovalState { Approved => "approved", Unapproved => "unapproved" }
);
open_string!(
    /// Identity of a local checkout or worktree.
    RootIdentity {
        Bound => "bound", Unbound => "unbound", Unverifiable => "unverifiable",
        Replaced => "replaced", Retired => "retired",
    }
);
open_string!(
    /// Kind of a machine-neutral root key.
    RootKeyKind { Remote => "remote", Label => "label" }
);
open_string!(
    /// Kind of checkout identity, distinct from a process incarnation.
    RootIncarnationKind { Token => "token" }
);
open_string!(
    /// A local project or a remote workspace reference.
    RefKind { Local => "local", Remote => "remote" }
);
open_string!(
    /// Read refusals. The two unavailable spellings have different meanings.
    RefusalCode {
        InvalidRequest => "invalid_request", InvalidParams => "invalid_params",
        StorageUnavailable => "storage_unavailable", StoreUnavailable => "store_unavailable",
        StorageError => "storage_error", EncodeFailed => "encode_failed",
        ModuleDraining => "module_draining", UnknownMethod => "unknown_method",
    }
);

/// A missing member differs from a present value, including a present null.
///
/// On a containing field, use `default` and `skip_serializing_if =
/// "Presence::is_missing"` to preserve omission. Standalone `Missing` serializes
/// as null because a JSON value cannot represent an absent member.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Presence<T> {
    #[default]
    Missing,
    Present(T),
}

impl<T> Presence<T> {
    pub fn new(value: T) -> Self {
        Self::Present(value)
    }

    pub fn missing() -> Self {
        Self::Missing
    }

    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Presence<T> {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        T::deserialize(decoder).map(Self::Present)
    }
}

impl<T: Serialize> Serialize for Presence<T> {
    fn serialize<S: Serializer>(&self, encoder: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Missing => encoder.serialize_unit(),
            Self::Present(value) => value.serialize(encoder),
        }
    }
}

macro_rules! wire_struct {
    ($(#[$doc:meta])* $name:ident { $($(#[$attr:meta])* $field:ident: $ty:ty),* $(,)? }) => {
        $(#[$doc])*
        #[non_exhaustive]
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub struct $name { $($(#[$attr])* pub $field: $ty,)* }
        impl $name {
            /// Constructs data without checking cross-field writer invariants.
            #[allow(clippy::too_many_arguments)]
            pub fn new($($field: $ty),*) -> Self { Self { $($field,)* } }
        }
    };
}

/// Full operation envelope. Constructors always include `params`, even for `{}`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request<P> {
    pub method: String,
    pub params: P,
}

impl<P> Request<P> {
    pub fn new(method: impl Into<String>, params: P) -> Self {
        Self {
            method: method.into(),
            params,
        }
    }
}

wire_struct!(
    /// A captured checkout binding; its incarnation is a checkout token string.
    ExecutionBinding {
        root: String, incarnation: String, registration_epoch: String,
    }
);
wire_struct!(
    /// `resolve` accepts any absolute query path, not only registered roots.
    ResolveParams {
        #[serde(alias = "canonical_root")]
        canonical_root: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution_binding: Option<ExecutionBinding>,
    }
);
wire_struct!(ResolveProjectIdParams {
    #[serde(alias = "project_id")]
    project_id: String,
});
wire_struct!(EnumerateParams {
    #[serde(alias = "workspace_id", skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
});
wire_struct!(
    /// Owner and repository components, passed through without normalization.
    ResolveRemoteParams { owner: String, repo: String }
);

/// Successful reads carry the process incarnation inside `result`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Success<T> {
    pub result: T,
}

impl<T> Success<T> {
    pub fn new(result: T) -> Self {
        Self { result }
    }
}

wire_struct!(Refusal {
    code: RefusalCode,
    message: String,
    #[serde(default)]
    detail: Option<serde_json::Value>,
});

/// Untagged reply envelope; a valid success takes precedence if refusal keys
/// are also present. Unknown keys are ignored rather than used as a discriminator.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Outcome<T> {
    Success(Success<T>),
    Refusal(Refusal),
}

impl<T> Outcome<T> {
    pub fn success(result: T) -> Self {
        Self::Success(Success::new(result))
    }

    pub fn refusal(error: Refusal) -> Self {
        Self::Refusal(error)
    }
}

wire_struct!(
    /// Path resolution. The four root-mode fields are flattened, not nested.
    ResolveReply {
        project_id: String,
        workspace_id: Option<String>, workspace_root: Option<String>, project_name: Option<String>,
        via: Via, gone: bool, generation: i64, incarnation: String,
        canonical_root: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        worktree_of: Option<String>,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        matched_root: Presence<Option<String>>,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        source_root: Presence<Option<String>>,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        root_records: Presence<Vec<RootRecord>>,
        #[serde(default, skip_serializing_if = "Presence::is_missing")]
        binding_status: Presence<Option<BindingStatus>>,
    }
);
wire_struct!(ResolveProjectIdReply {
    project_id: String,
    via: Via,
    gone: bool,
    generation: i64,
    incarnation: String,
});
wire_struct!(EnumerateReply {
    workspaces: Vec<Workspace>, projects: Vec<Project>, generation: i64, incarnation: String,
});
wire_struct!(
    /// Remote lookup has a flattened status tag with shape-specific fields.
    ResolveRemoteReply {
        #[serde(flatten)]
        status: ResolveRemoteStatus,
        generation: i64,
        incarnation: String,
    }
);
wire_struct!(
    /// A unique matching project and one of its registered canonical roots.
    ResolveRemoteFound { project_id: String, root: String }
);
wire_struct!(
    /// Matching project ids, sorted lexicographically by the current writer.
    ResolveRemoteAmbiguous { project_ids: Vec<String> }
);

/// The status tag selects the required fields; a miss and ambiguity are successes.
/// Unknown status text is preserved, while unfamiliar object members are ignored.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveRemoteStatus {
    Found(ResolveRemoteFound),
    None,
    Ambiguous(ResolveRemoteAmbiguous),
    Other(String),
}

impl ResolveRemoteStatus {
    pub fn found(project_id: String, root: String) -> Self {
        Self::Found(ResolveRemoteFound::new(project_id, root))
    }

    pub fn ambiguous(project_ids: Vec<String>) -> Self {
        Self::Ambiguous(ResolveRemoteAmbiguous::new(project_ids))
    }

    pub fn other(status: impl Into<String>) -> Self {
        Self::Other(status.into())
    }
}

impl Serialize for ResolveRemoteStatus {
    fn serialize<S: Serializer>(&self, encoder: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;

        let mut map = encoder.serialize_map(None)?;
        match self {
            Self::Found(found) => {
                map.serialize_entry("status", "found")?;
                map.serialize_entry("projectId", &found.project_id)?;
                map.serialize_entry("root", &found.root)?;
            }
            Self::None => map.serialize_entry("status", "none")?,
            Self::Ambiguous(ambiguous) => {
                map.serialize_entry("status", "ambiguous")?;
                map.serialize_entry("projectIds", &ambiguous.project_ids)?;
            }
            Self::Other(status) => map.serialize_entry("status", status)?,
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for ResolveRemoteStatus {
    fn deserialize<D: Deserializer<'de>>(decoder: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let mut fields = serde_json::Map::<String, serde_json::Value>::deserialize(decoder)?;
        let status = fields
            .remove("status")
            .ok_or_else(|| D::Error::missing_field("status"))?;
        let status = String::deserialize(status).map_err(D::Error::custom)?;
        let payload = serde_json::Value::Object(fields);
        match status.as_str() {
            "found" => ResolveRemoteFound::deserialize(payload)
                .map(Self::Found)
                .map_err(D::Error::custom),
            "none" => Ok(Self::None),
            "ambiguous" => ResolveRemoteAmbiguous::deserialize(payload)
                .map(Self::Ambiguous)
                .map_err(D::Error::custom),
            _ => Ok(Self::Other(status)),
        }
    }
}

wire_struct!(Workspace {
    workspace_id: String, name: String, root: Option<String>,
});
wire_struct!(Project {
    project_id: String, name: String, roots: Vec<String>, implicit: bool, ref_kind: RefKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    device_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_route_activity_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_records: Option<Vec<RootRecord>>,
});
wire_struct!(RootRecord {
    root: String, remotes: Vec<GitRemote>,
    #[serde(skip_serializing_if = "Option::is_none")]
    remotes_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_key: Option<RootKey>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_key_mismatch: Option<bool>,
    incarnation: Option<RootIncarnation>, registration_epoch: Option<String>,
    identity: RootIdentity, approval: Approval,
});
wire_struct!(RootIncarnation {
    kind: RootIncarnationKind,
    value: String
});
wire_struct!(Approval { state: ApprovalState, journal_seq: Option<i64> });
wire_struct!(GitRemote {
    name: String,
    owner: String,
    repo: String,
    owned: bool
});
wire_struct!(RootKey {
    kind: RootKeyKind,
    root_key: String
});
