//! Usable input-token ceilings and the measurement source for current fill.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Geometry {
    pub usable_soft: u64,
    pub usable_hard: u64,
    pub absolute_wall: u64,
    /// Describes where the ceilings came from. Accept unfamiliar labels;
    /// they do not change how the ceilings are interpreted.
    pub derivation: String,
}

impl Geometry {
    pub fn is_valid(&self, context_window: Option<u64>) -> bool {
        self.usable_soft > 0
            && self.usable_soft <= self.usable_hard
            && self.usable_hard <= self.absolute_wall
            && context_window.is_none_or(|window| self.absolute_wall <= window)
    }
}

/// A measured or explicitly estimated input count and its source. Omit the
/// entire status field when there is no reading; do not manufacture zero fill.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Fill {
    pub input_tokens: u64,
    /// Open attribution label, for example `measured` or `estimate`.
    pub basis: String,
    /// The provider's request id for the response whose input was measured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observed_request_id: Option<String>,
}
