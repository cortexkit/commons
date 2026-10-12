//! Optional Claude Code negotiation and connection-scoped admission.
//!
//! None of these checks authenticates a caller. The transport supplies the
//! daemon-attested principal and connection identity, never request body data.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::hook::DEFAULT_HOOK_CAP_BYTES;

pub const CAPABILITY: &str = "claude-code-step/v1";
pub const PRINCIPAL: &str = "reserved:thalamus";
/// The 3 MiB ceiling for encoded Claude Code request/reply envelopes. Reuse
/// the host hook ceiling so hooks, statuses and view replies share one bound;
/// accepted envelopes must be strictly smaller than this limit.
pub const MAX_ENVELOPE_BYTES: usize = DEFAULT_HOOK_CAP_BYTES;

/// A request-parameter error mapped to the role's `invalid_params` error reply,
/// with `detail.field` identifying the parameter. It is not a compaction
/// `refuse` answer: whether those answers may be retried remains determined
/// by their role refusal code, not by this field-validation result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvalidParams {
    pub field: &'static str,
}

impl InvalidParams {
    pub fn new(field: &'static str) -> Self {
        Self { field }
    }

    pub fn code(&self) -> &'static str {
        "invalid_params"
    }
}

/// Byte caps count the encoded envelope, including raw payload escaping and
/// sidecar overhead. A single message is never truncated to fit these caps.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PayloadCaps {
    pub ingress_envelope_bytes: usize,
    pub hook_envelope_bytes: usize,
    pub single_message_bytes: usize,
    pub view_page_bytes: usize,
}

/// Capability values advertised by `role.describe` and selected unchanged by
/// `transform.declare`. Once the plan is accepted, retain the same selection
/// for that connection; later requests cannot revise it (CONTRACT.md §14.1).
/// Version counters live in `epochs`; generation ids live in
/// `profile_generation` and `provider_generation`. Keep them out of the
/// `capability` name, which remains an identifier such as `claude-code-step/v1`.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ClaudeCodeCapabilities {
    pub capability: String,
    pub principal: String,
    pub harness: String,
    pub profile: String,
    pub profile_generation: String,
    pub provider_generation: String,
    pub observation: String,
    pub commons_revision: String,
    pub wire_majors: BTreeMap<String, String>,
    pub package_versions: BTreeMap<String, String>,
    pub epochs: BTreeMap<String, u64>,
    pub exact_endpoint: String,
    pub partial_message_cut: bool,
    pub caps: PayloadCaps,
    pub raw_view_read: bool,
    pub digest: String,
    pub cache_policy_revision: String,
    pub canonical_reduce: bool,
    pub guidance: bool,
    pub notice_dispatch_ack: bool,
    pub bootstrap_complete: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl ClaudeCodeCapabilities {
    pub fn validate(&self) -> Result<(), InvalidParams> {
        let caps = &self.caps;
        if self.capability != CAPABILITY
            || !is_valid_capability_identifier(&self.capability)
            || self
                .capabilities
                .iter()
                .any(|id| !is_valid_capability_identifier(id))
            || self.principal != PRINCIPAL
            || self.harness != "claude-code"
            || self.observation != "answer"
            || self.commons_revision.is_empty()
            || self.cache_policy_revision.is_empty()
            || [
                "codec",
                "raw_provenance",
                "render",
                "tagger",
                "memory",
                "compartment",
            ]
            .iter()
            .any(|epoch| !self.epochs.contains_key(*epoch))
            || ["compaction-provider", "step-transform-provider"]
                .iter()
                .any(|role| {
                    self.wire_majors.get(*role).map(String::as_str) != Some("v1")
                        || self
                            .package_versions
                            .get(*role)
                            .is_none_or(String::is_empty)
                })
            || self.profile.is_empty()
            || self.profile_generation.is_empty()
            || self.provider_generation.is_empty()
            || self.exact_endpoint != "native_top_level_inclusive"
            || self.digest != "sha256"
            || caps.ingress_envelope_bytes == 0
            || caps.ingress_envelope_bytes > MAX_ENVELOPE_BYTES
            || caps.hook_envelope_bytes == 0
            || caps.hook_envelope_bytes > MAX_ENVELOPE_BYTES
            || caps.single_message_bytes == 0
            || caps.single_message_bytes >= caps.ingress_envelope_bytes
            || caps.single_message_bytes >= caps.hook_envelope_bytes
            || caps.view_page_bytes == 0
            || caps.view_page_bytes >= MAX_ENVELOPE_BYTES
        {
            return Err(InvalidParams::new("claude_code"));
        }
        Ok(())
    }
}

/// State kept by the provider after a proposed plan is accepted, recording
/// which Claude Code capabilities were chosen for one connection. It is not
/// received as request-body data. Construct it with the caller principal from
/// the transport after the daemon authenticates the connection (verifies who
/// opened it), and the accepted plan's capability selection. Body harnesses,
/// tags and plan fields are not identity proofs (CONTRACT.md §14.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeCodeLane {
    connection_id: String,
    selected: Option<ClaudeCodeCapabilities>,
}

impl ClaudeCodeLane {
    pub fn admit(
        attested_principal: &str,
        connection_id: impl Into<String>,
        selected: Option<ClaudeCodeCapabilities>,
    ) -> Result<Self, InvalidParams> {
        if let Some(bundle) = &selected {
            if attested_principal != PRINCIPAL {
                return Err(InvalidParams::new("claude_code"));
            }
            bundle.validate()?;
        }
        Ok(Self {
            connection_id: connection_id.into(),
            selected,
        })
    }

    /// Only `reserved:thalamus` may select the Claude Code capability. If its
    /// plan selected none, return `None` so Claude Code-only checks, such as
    /// required geometry and raw-message sidecars, are not enabled. A selected
    /// bundle is available only to the original authenticated principal and
    /// connection, not to another connection.
    pub fn selected_for(
        &self,
        attested_principal: &str,
        connection_id: &str,
    ) -> Result<Option<&ClaudeCodeCapabilities>, InvalidParams> {
        if self.selected.is_some()
            && (attested_principal != PRINCIPAL || connection_id != self.connection_id)
        {
            return Err(InvalidParams::new("claude_code"));
        }
        Ok(self.selected.as_ref())
    }

    /// Call when processing `compaction.setup` (which configures the session's
    /// initial model view) and on any attempted plan replacement. Compare the
    /// request's capability selection with the one kept when its plan was
    /// accepted: later declarations cannot change it or enable Claude Code
    /// features on a connection whose accepted plan selected none.
    pub fn check_frozen(
        &self,
        selected: Option<&ClaudeCodeCapabilities>,
    ) -> Result<(), InvalidParams> {
        if self.selected.as_ref() != selected {
            return Err(InvalidParams::new("claude_code"));
        }
        Ok(())
    }
}

/// The daemon's `<name>/v<N>` grammar: an ASCII kebab name up to 64 bytes
/// and a positive, non-zero-padded version bounded by `u32::MAX`.
/// A dev-only parity test compares this checker with subc-protocol's HELLO
/// validator so a dependency update cannot silently change admission behavior.
pub fn is_valid_capability_identifier(identifier: &str) -> bool {
    if identifier.chars().any(char::is_whitespace) {
        return false;
    }
    let Some((name, version)) = identifier.split_once("/v") else {
        return false;
    };
    if name.is_empty() || name.len() > 64 || version.is_empty() {
        return false;
    }
    let name_bytes = name.as_bytes();
    if !name_bytes[0].is_ascii_lowercase()
        || (name.len() > 1
            && !name_bytes[name.len() - 1].is_ascii_lowercase()
            && !name_bytes[name.len() - 1].is_ascii_digit())
        || name_bytes.windows(2).any(|pair| pair == b"--")
        || !name_bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
        || (version.len() > 1 && version.starts_with('0'))
        || !version.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    matches!(version.parse::<u64>(), Ok(value) if (1..=u64::from(u32::MAX)).contains(&value))
}

/// Check presence and JSON syntax without re-encoding the sidecar. The native
/// provider codec separately verifies equality to the parsed message after
/// removing only Anthropic transport-level `cache_control` members at the
/// positions listed in this crate's CONTRACT.md §14.3.
pub fn check_raw_message(raw_message: Option<&str>) -> Result<(), InvalidParams> {
    let raw = raw_message
        .filter(|raw| !raw.is_empty())
        .ok_or_else(|| InvalidParams::new("raw_message"))?;
    serde_json::from_str::<serde_json::Value>(raw)
        .map_err(|_| InvalidParams::new("raw_message"))?;
    Ok(())
}
