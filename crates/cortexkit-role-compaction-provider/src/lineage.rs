//! Lineage references and the descent rule for Setup.
//!
//! A lineage is one line of a conversation's history, with its own ordinals.
//! A session that continues another lineage (a fork, say) declares the
//! inherited prefix on `compaction.setup` with `descends_from`. The provider
//! keeps one link per lineage: it validates the retained history and saves
//! the link durably before replying. Links are restored only from the
//! provider's own records, never from caller JSON: a caller cannot certify
//! that the provider holds a prefix.

use serde::{Deserialize, Serialize};

use crate::errors;

/// History inherited by the current lineage: the parent lineage and the last
/// ordinal inherited, inclusive.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct DescendsFrom {
    pub lineage_id: String,
    pub through_ordinal: u64,
}

impl DescendsFrom {
    pub fn new(lineage_id: impl Into<String>, through_ordinal: u64) -> Self {
        Self {
            lineage_id: lineage_id.into(),
            through_ordinal,
        }
    }
}

/// A message the runner wrote, by its place in the lineage.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct MessageRef {
    pub ordinal: u64,
    /// The runner's opaque message id.
    pub mid: String,
}

/// The descent saved for one lineage; absent for a root lineage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DescentLink {
    descends_from: Option<DescendsFrom>,
}

/// Why a declaration was refused. A refused declaration never changes the
/// saved link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescentProblem {
    /// The parent, the cut, or a message of the retained prefix is not held.
    HistoryUnreadable,
    /// The declaration differs from the saved link.
    Conflict,
}

impl DescentProblem {
    /// The refusal code: `history_unreadable` in a `refuse` answer, or
    /// `invalid_params` as an `ERROR`.
    pub fn code(self) -> &'static str {
        match self {
            Self::HistoryUnreadable => errors::refuse_codes::HISTORY_UNREADABLE,
            Self::Conflict => errors::INVALID_PARAMS,
        }
    }

    /// The field an `invalid_params` refusal names.
    pub fn field(self) -> Option<&'static str> {
        match self {
            Self::HistoryUnreadable => None,
            Self::Conflict => Some("lineage_id"),
        }
    }
}

impl DescentLink {
    pub fn descends_from(&self) -> Option<&DescendsFrom> {
        self.descends_from.as_ref()
    }

    /// Validate and save the first declaration; absence and identical repeats
    /// (a Setup repeated after a failure) are no-ops. `holds_prefix` must check
    /// that the named parent and cut exist in this conversation and that every
    /// retained message through the cut is held. It is not called for an
    /// absent, repeated or conflicting declaration.
    pub fn declare(
        &mut self,
        declaration: Option<&DescendsFrom>,
        holds_prefix: impl FnOnce(&DescendsFrom) -> bool,
    ) -> Result<(), DescentProblem> {
        let Some(declaration) = declaration else {
            return Ok(());
        };
        if let Some(saved) = &self.descends_from {
            return if saved == declaration {
                Ok(())
            } else {
                Err(DescentProblem::Conflict)
            };
        }
        if !holds_prefix(declaration) {
            return Err(DescentProblem::HistoryUnreadable);
        }
        self.descends_from = Some(declaration.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{setup::SetupRequest, vectors};
    use serde_json::Value;

    fn check_vectors(conflicts: bool) -> usize {
        let file = vectors::load("setup-descent.json");
        let mut checked = 0;
        for case in vectors::cases(&file, "checks") {
            let calls = case["calls"].as_array().unwrap();
            let has_conflict = calls
                .iter()
                .any(|call| call["refusal"]["code"] == "invalid_params");
            if has_conflict != conflicts {
                continue;
            }
            checked += 1;
            let name = case["name"].as_str().unwrap();
            let mut link: DescentLink = serde_json::from_value(serde_json::json!({
                "descends_from": case["provider_state"]["descends_from"]
            }))
            .unwrap();
            for call in calls {
                assert_eq!(call["method"], "compaction.setup", "{name}");
                let requested =
                    vectors::round_trip::<SetupRequest>(name, &call["request"]).descends_from;
                let before = link.clone();
                let mut history_checks = 0;
                let result = link.declare(requested.as_ref(), |declaration| {
                    history_checks += 1;
                    let Some(held) =
                        case["provider_state"]["parents"][&declaration.lineage_id].as_array()
                    else {
                        return false;
                    };
                    // Fixture ordinals stand for durable originals in the parent.
                    // Checking the whole prefix catches an interior history gap,
                    // not just an absent parent or an unheld endpoint.
                    (0..=declaration.through_ordinal)
                        .all(|ordinal| held.contains(&Value::from(ordinal)))
                });
                if let Some(refusal) = call.get("refusal") {
                    let problem = result.expect_err(name);
                    assert_eq!(problem.code(), refusal["code"].as_str().unwrap(), "{name}");
                    assert_eq!(
                        problem.field(),
                        refusal["detail"]["field"].as_str(),
                        "{name}"
                    );
                    assert_eq!(link, before, "{name}: refusal changed the link");
                } else {
                    assert_eq!(call["outcome"], "accepted", "{name}");
                    assert_eq!(result, Ok(()), "{name}");
                }
                assert_eq!(
                    history_checks,
                    call["history_checks"].as_u64().unwrap(),
                    "{name}"
                );
                assert_eq!(
                    serde_json::to_value(link.descends_from()).unwrap(),
                    call["saved_descends_from"],
                    "{name}"
                );
                // The next call sees only the provider's persisted link, not
                // an in-memory declaration supplied by the preceding caller.
                link = serde_json::from_value(serde_json::to_value(&link).unwrap()).unwrap();
            }
        }
        checked
    }

    #[test]
    fn descent_vectors_validate_history_and_repeat_idempotently() {
        assert_eq!(check_vectors(false), 4);
    }

    #[test]
    fn descent_vectors_refuse_conflicting_lineage_declarations() {
        assert_eq!(check_vectors(true), 2);
    }
}
