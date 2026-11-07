//! Pure decisions for retaining keyed outcomes through the existing late log.

use crate::{catalog::CatalogTool, errors};
use serde::{Deserialize, Serialize};
use subc_protocol::ErrorBody;

/// Session capability and route-bind consumer opt-in.
pub const CAPABILITY: &str = "reply_custody";

/// Catalog execution semantics. Unknown values round-trip conservatively.
/// Providers should match their module manifest's declaration for the tool.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(from = "String", into = "String")]
#[non_exhaustive]
pub enum ExecutionMode {
    Pure,
    Mutating,
    Unfenceable,
    Unknown(String),
}

impl From<String> for ExecutionMode {
    fn from(value: String) -> Self {
        match value.as_str() {
            "pure" => Self::Pure,
            "mutating" => Self::Mutating,
            "unfenceable" => Self::Unfenceable,
            _ => Self::Unknown(value),
        }
    }
}

impl From<ExecutionMode> for String {
    fn from(value: ExecutionMode) -> Self {
        match value {
            ExecutionMode::Pure => "pure".into(),
            ExecutionMode::Mutating => "mutating".into(),
            ExecutionMode::Unfenceable => "unfenceable".into(),
            ExecutionMode::Unknown(value) => value,
        }
    }
}

/// Refuse a custody declaration missing either dependency, naming it in detail.
/// The booleans are declarations, not whether the implementation knows a name.
pub fn check_dependencies(
    reply_custody: bool,
    late_results: bool,
    call_key: bool,
) -> Result<(), ErrorBody> {
    if reply_custody {
        for (name, declared) in [("late_results", late_results), ("call_key", call_key)] {
            if !declared {
                return Err(errors::invalid_request(
                    "capabilities",
                    format!("reply_custody requires {name}"),
                ));
            }
        }
    }
    Ok(())
}

/// Inspect only the catalog entry frozen for the run, never a live catalog.
pub fn is_pure(frozen_tool: &CatalogTool) -> bool {
    frozen_tool.execution_mode == Some(ExecutionMode::Pure)
}

/// Whether a terminal entry is required. A held call keeps its existing late
/// result obligation even when the route did not opt in or the tool is Pure.
/// A false return for a Pure direct reply permits omission; it does not forbid
/// keeping an entry. Unkeyed calls cannot be correlated in the late log.
/// `direct_reply_sent` means written to the route before it closed, to the
/// provider's knowledge, not a claim that the custodian committed the reply.
pub fn entry_required(
    route_opted_in: bool,
    keyed: bool,
    first_reply_held: bool,
    direct_reply_sent: bool,
    frozen_tool: &CatalogTool,
) -> bool {
    keyed && (first_reply_held || (route_opted_in && (!is_pure(frozen_tool) || !direct_reply_sent)))
}

/// Whether ordinary route closure must leave the call running. Scope end,
/// Stop, cancel and withdrawal are separate operations with unchanged rules.
pub fn survives_route_close(route_opted_in: bool, keyed: bool, frozen_tool: &CatalogTool) -> bool {
    route_opted_in && keyed && !is_pure(frozen_tool)
}

/// Read the route bind's opt-in, never a per-call argument.
pub fn route_opts_in(consumer_capabilities: Option<&[String]>) -> bool {
    consumer_capabilities.is_some_and(|caps| caps.iter().any(|cap| cap == CAPABILITY))
}
