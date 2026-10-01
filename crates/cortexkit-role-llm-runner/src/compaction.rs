//! `compaction.ready`: a compaction provider's signal to a runner.
//!
//! The compaction interface itself (Setup, the per-step status, the
//! answers `NOOP`, CompactionMessage, `WAIT` and `REFUSE`) is a set of calls
//! the runner makes to the session's compaction provider; the
//! `compaction-provider/v1` role defines those shapes. The one op the runner
//! serves is `compaction.ready`: when a provider that answered `WAIT`
//! finishes its work, it tells the runner, which asks again with a fresh
//! status instead of waiting out the bound. It is a hint: a runner that
//! never receives it asks again when the `WAIT`'s bound runs out.
//!
//! It arrives on the provider's module-level route, which is not bound to a
//! session, so it names the session.

use serde::{Deserialize, Serialize};

/// The `compaction.ready` request. Decoded leniently: the answer is the
/// same whatever else it carries.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CompactionReady {
    /// The session the provider was asked about, spelled exactly as the
    /// runner's status named it.
    pub session: String,
}

impl CompactionReady {
    pub fn new(session: impl Into<String>) -> Self {
        Self {
            session: session.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn compaction_ready_vectors_decode() {
        let file = vectors::load("compaction-ready.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<CompactionReady>(
                case["name"].as_str().unwrap(),
                &case["request"],
            );
        }
        for case in vectors::cases(&file, "tolerated") {
            let name = case["name"].as_str().unwrap();
            serde_json::from_value::<CompactionReady>(case["request"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        assert_eq!(CompactionReady::new("s").session, "s");
    }
}
