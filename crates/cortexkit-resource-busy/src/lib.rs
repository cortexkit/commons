//! Shared wire shape for a `resource_busy` refusal.
//!
//! A provider returns this refusal only before doing anything for the request:
//! nothing has been posted, launched, navigated or dispatched. A caller may
//! therefore record the call as provably unsent and retry. It waits no longer
//! than `retry_after_ms` and never past its own call deadline; after that
//! deadline it reports a `resource_busy` failure, not a timeout.
//!
//! The holder is always the same agent as the caller (its head or one of its
//! flows), never another agent's holder. That restriction prevents the reply
//! from leaking information across agents. The provider owns and bounds the
//! lease, releasing it at session or run end, at scope end, or on an idle
//! timeout; it is never an unbounded lock.
//!
//! Resource names are open strings, constrained to 1 through 64 characters
//! from `[a-z0-9_]`, so providers can add names without a release. This shared
//! refusal is independent of any role and applies to plain operations as well
//! as tool calls.

#![forbid(unsafe_code)]

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

/// Error code for the refusal represented by [`ResourceBusy`].
pub const RESOURCE_BUSY: &str = "resource_busy";

/// A refusal to acquire an exclusive resource currently held by another
/// caller of the same agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ResourceBusy {
    /// Open provider-defined resource name, such as `browser_profile` or
    /// `pointer`.
    pub resource: String,
    /// The same agent's caller that currently holds the resource.
    pub holder: Holder,
    /// Unix milliseconds when the holder's lease began.
    pub since_ms: u64,
    /// Hint for how long until the lease expires or idles out, whichever is
    /// sooner.
    pub retry_after_ms: u64,
}

impl ResourceBusy {
    /// Create a resource-busy detail. Call [`validate`](Self::validate) before
    /// sending externally supplied values.
    pub fn new(
        resource: impl Into<String>,
        holder: Holder,
        since_ms: u64,
        retry_after_ms: u64,
    ) -> Self {
        Self {
            resource: resource.into(),
            holder,
            since_ms,
            retry_after_ms,
        }
    }

    /// Check the resource name and the holder's `flow_id`/kind combination.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.resource.is_empty()
            || self.resource.len() > 64
            || !self
                .resource
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        {
            return Err(ValidationError::InvalidResource);
        }

        match (&self.holder.kind, &self.holder.flow_id) {
            (HolderKind::Flow, None) => Err(ValidationError::MissingFlowId),
            (HolderKind::Flow, Some(_)) => Ok(()),
            (_, Some(_)) => Err(ValidationError::UnexpectedFlowId),
            (_, None) => Ok(()),
        }
    }
}

/// The holder in the caller's own agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Holder {
    /// Holder kind, preserving values from newer protocol versions.
    #[serde(
        serialize_with = "serialize_kind",
        deserialize_with = "deserialize_kind"
    )]
    pub kind: HolderKind,
    /// Present exactly when `kind` is [`HolderKind::Flow`].
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub flow_id: Option<String>,
}

impl Holder {
    /// Create a head-session holder.
    pub fn head() -> Self {
        Self {
            kind: HolderKind::Head,
            flow_id: None,
        }
    }

    /// Create a flow holder with its flow identifier.
    pub fn flow(id: impl Into<String>) -> Self {
        Self {
            kind: HolderKind::Flow,
            flow_id: Some(id.into()),
        }
    }
}

/// A known holder kind or a newer kind preserved verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum HolderKind {
    /// The agent's head session.
    Head,
    /// A flow belonging to the same agent.
    Flow,
    /// A kind introduced by a newer protocol version.
    Other(String),
}

fn serialize_kind<S>(kind: &HolderKind, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match kind {
        HolderKind::Head => serializer.serialize_str("head"),
        HolderKind::Flow => serializer.serialize_str("flow"),
        HolderKind::Other(value) => serializer.serialize_str(value),
    }
}

fn deserialize_kind<'de, D>(deserializer: D) -> Result<HolderKind, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    Ok(match value.as_str() {
        "head" => HolderKind::Head,
        "flow" => HolderKind::Flow,
        _ => HolderKind::Other(value),
    })
}

/// Why a resource-busy detail does not match the wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationError {
    /// Resource names must contain 1 to 64 lowercase ASCII letters, digits,
    /// or underscores.
    InvalidResource,
    /// A flow holder must have a flow identifier.
    MissingFlowId,
    /// A non-flow holder must not have a flow identifier.
    UnexpectedFlowId,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidResource => write!(
                f,
                "resource must contain 1 to 64 lowercase ASCII letters, digits, or underscores"
            ),
            Self::MissingFlowId => write!(f, "a flow holder must have a flow_id"),
            Self::UnexpectedFlowId => {
                write!(f, "only a flow holder may have a flow_id")
            }
        }
    }
}

impl std::error::Error for ValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_id_combination_rule_is_enforced() {
        let missing_flow_id = ResourceBusy::new(
            "browser_profile",
            Holder {
                kind: HolderKind::Flow,
                flow_id: None,
            },
            1,
            2,
        );
        assert_eq!(
            missing_flow_id.validate(),
            Err(ValidationError::MissingFlowId)
        );

        let head_with_flow_id = ResourceBusy::new(
            "browser_profile",
            Holder {
                kind: HolderKind::Head,
                flow_id: Some("flow-1".into()),
            },
            1,
            2,
        );
        assert_eq!(
            head_with_flow_id.validate(),
            Err(ValidationError::UnexpectedFlowId)
        );
    }

    #[test]
    fn resource_name_must_match_the_open_string_constraint() {
        for resource in ["", "Browser", "pointer-name", "two words", "é"] {
            let detail = ResourceBusy::new(resource, Holder::head(), 1, 2);
            assert_eq!(
                detail.validate(),
                Err(ValidationError::InvalidResource),
                "{resource:?}"
            );
        }
        let too_long = "a".repeat(65);
        assert_eq!(
            ResourceBusy::new(too_long, Holder::head(), 1, 2).validate(),
            Err(ValidationError::InvalidResource)
        );
        assert!(ResourceBusy::new("pointer_2", Holder::head(), 1, 2)
            .validate()
            .is_ok());
    }
}

#[cfg(test)]
mod kind_serde_tests {
    use super::*;

    #[test]
    fn unknown_holder_kind_round_trips_unchanged() {
        let json = r#"{"resource":"browser_profile","holder":{"kind":"remote_worker"},"since_ms":100,"retry_after_ms":250}"#;
        let detail: ResourceBusy = serde_json::from_str(json).unwrap();
        assert_eq!(
            detail.holder.kind,
            HolderKind::Other("remote_worker".into())
        );
        assert_eq!(serde_json::to_string(&detail).unwrap(), json);
    }
}
