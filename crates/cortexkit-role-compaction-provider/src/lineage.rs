//! The shared descent rule for setup, step and hook declarations.
//!
//! A provider keeps one link per session lineage in the same conversation.
//! It must validate retained history and durably save the resulting link
//! before replying. Restore links only from provider-owned records, never
//! caller JSON: a caller cannot certify that the provider holds a prefix.

use serde::{Deserialize, Serialize};

use crate::{errors, status::DescendsFrom};

/// An undeclared lineage can still acquire descent on a later step or hook.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct DescentLink {
    descends_from: Option<DescendsFrom>,
}

/// A rejected declaration never changes the saved link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DescentProblem {
    HistoryUnreadable,
    Conflict,
}

impl DescentProblem {
    pub fn code(self) -> &'static str {
        match self {
            Self::HistoryUnreadable => errors::refuse_codes::HISTORY_UNREADABLE,
            Self::Conflict => errors::INVALID_PARAMS,
        }
    }

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
    /// are no-ops. `holds_prefix` must check the named parent and cut exist in
    /// this conversation and every retained message through the cut is held.
    /// It is not called for an absent, repeated or conflicting declaration.
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
    use crate::{setup::SetupRequest, status::StepStatus, vectors};
    use cortexkit_role_step_transform_provider::hook::HookCall;
    use serde_json::Value;

    fn declaration(name: &str, call: &Value) -> Option<DescendsFrom> {
        match call["method"].as_str().unwrap() {
            "compaction.setup" => {
                vectors::round_trip::<SetupRequest>(name, &call["request"]).descends_from
            }
            "compaction.step" => {
                vectors::round_trip::<StepStatus>(name, &call["request"]).descends_from
            }
            "transform.hook" => {
                let hook = vectors::round_trip::<HookCall>(name, &call["request"]);
                hook.descends_from
                    .map(|link| DescendsFrom::new(link.lineage_id, link.through_ordinal))
            }
            method => panic!("{name}: unknown method {method}"),
        }
    }

    fn check_vectors(conflicts: bool) {
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
                let requested = declaration(name, call);
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
        assert_eq!(checked, if conflicts { 5 } else { 8 });
    }

    #[test]
    fn descent_vectors_validate_history_and_preserve_step_and_hook_paths() {
        check_vectors(false);
    }

    #[test]
    fn descent_vectors_refuse_conflicting_lineage_declarations() {
        check_vectors(true);
    }
}
