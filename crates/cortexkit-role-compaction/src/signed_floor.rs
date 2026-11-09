//! The runner's durable boundary for signed reasoning invalidated by tool changes.

use serde::{Deserialize, Serialize};

/// Signed parts at or before this entry are already absent from model requests.
/// Providers declaring `compaction-signed-floor/v1` do not count, hold for, or
/// plan those parts. The boundary is runner state, not part of a view or checksum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SignedFloor {
    /// Stable entry id on the session's active path, never an ordinal.
    pub through: String,
}

impl SignedFloor {
    pub fn new(through: impl Into<String>) -> Self {
        Self {
            through: through.into(),
        }
    }
}
