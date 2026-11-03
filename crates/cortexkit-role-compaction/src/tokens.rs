//! W4.11 count metadata, outside all transcript checksums. These types do not
//! validate conformance: decoding checks shapes and W1.2 integer bounds only;
//! runners and readers enforce coverage, family, stability and exactness rules.
//! A tool part's `tokens` may decode as either shape, though conforming runners
//! send the tool shape only once its result exists.
//!
//! Readers adopt each response's `token_family` in receipt order, discard held
//! counts from other families and estimate absent counts rather than using zero.

use serde::{Deserialize, Serialize};

/// A text or file part's full-content count, excluding message framing and
/// view edits. Text includes any rendered tag prefix. Images use the reading
/// model's image rule; other media omit counts unless the runner has a rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TokenCount {
    #[serde(deserialize_with = "crate::integer::required")]
    pub count: u64,
}

impl TokenCount {
    pub fn new(count: u64) -> Self {
        Self { count }
    }
}

/// Separate full-content counts let a provider estimate the saving from
/// replacing the tool's output, keeping the call/result structure.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ToolTokens {
    /// Tool name plus arguments as rendered, without message framing.
    #[serde(deserialize_with = "crate::integer::required")]
    pub input: u64,
    /// Everything in the rendered result: text, delivered text and images
    /// by the reading model's image rule, without message framing.
    #[serde(deserialize_with = "crate::integer::required")]
    pub output: u64,
}

impl ToolTokens {
    pub fn new(input: u64, output: u64) -> Self {
        Self { input, output }
    }
}

/// Per-part metadata shapes. Conforming runners use `Count` for text/files
/// and `Tool` for tools with results; reasoning and opaque parts omit counts.
/// Decoding does not enforce this kind/shape relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum PartTokens {
    Count(TokenCount),
    Tool(ToolTokens),
}

impl PartTokens {
    pub fn count(count: u64) -> Self {
        Self::Count(TokenCount::new(count))
    }

    pub fn tool(input: u64, output: u64) -> Self {
        Self::Tool(ToolTokens::new(input, output))
    }
}

/// An assistant step's reasoning count, not a per-reasoning-part count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ReasoningTokens {
    #[serde(deserialize_with = "crate::integer::required")]
    pub count: u64,
    /// True only after checking a byte-prefix extension of final provider
    /// prompts, excluding sampling settings (W4.11). Runners MUST freeze a
    /// measured value within its entry and family; providers may rely on it.
    /// After a family change, remeasure or omit it. False is a revisable
    /// generation-side estimate, using separately reported reasoning first.
    pub measured: bool,
}

impl ReasoningTokens {
    pub fn new(count: u64, measured: bool) -> Self {
        Self { count, measured }
    }
}

/// A notice's system-text or tool-definition count for the next request under
/// its conditions. The system count excludes system entries. Never sent in rides.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SurfaceTokens {
    #[serde(deserialize_with = "crate::integer::required")]
    pub count: u64,
    /// True if the counted bytes are those the provider receives, false if
    /// the host renders the runner's input. Providers may report inexact
    /// counts but SHOULD NOT use them for drop decisions.
    pub exact: bool,
}

impl SurfaceTokens {
    pub fn new(count: u64, exact: bool) -> Self {
        Self { count, exact }
    }
}
