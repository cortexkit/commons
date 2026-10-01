//! `session.baseline`: what the session froze, and what has changed since.
//!
//! Required of every runner and answered only to the session's owner (any
//! other caller is refused `scope_owner_mismatch`). The admission reply
//! carries the same facts as a hint; after a lost reply, or after a prefix
//! rebuild applied a change, the owner reads `session.baseline` instead of
//! guessing. A runner that has not yet seen the session's first request
//! answers `not_yet`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The `session.baseline` request. It takes no fields, and refuses any.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BaselineRequest {}

/// Answer states. A decoder keeps any other value as a plain string
/// rather than failing.
pub mod states {
    /// The runner has not yet seen the session's first request, so it has
    /// no baseline. The owner skips its change checks until it has one.
    pub const NOT_YET: &str = "not_yet";
    /// `baseline` carries the session's baseline.
    pub const READY: &str = "ready";
}

/// The `session.baseline` answer. Decoded leniently.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct BaselineReply {
    /// One of [`states`]; any other value decodes as a plain string.
    pub state: String,
    /// Present exactly when `state` is `ready`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<Baseline>,
}

impl BaselineReply {
    pub fn not_yet() -> Self {
        Self {
            state: states::NOT_YET.into(),
            baseline: None,
        }
    }

    pub fn ready(baseline: Baseline) -> Self {
        Self {
            state: states::READY.into(),
            baseline: Some(baseline),
        }
    }

    pub fn is_not_yet(&self) -> bool {
        self.state == states::NOT_YET
    }
}

/// A session's baseline: the same facts in the admission reply and in
/// `session.baseline`.
///
/// Non-exhaustive so later optional members are additive: use
/// [`Baseline::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Baseline {
    /// Each fetched plan item's frozen digest and preflight digest.
    pub items: Vec<BaselineItem>,
    /// Optional plan items recorded as absent at admission.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absent_items: Vec<AbsentItem>,
    /// The composition digest of the latest prefix rebuild, or of the start
    /// record before any: with `tool_names`, the frozen-prefix digest.
    pub composition_digest: String,
    /// The model-facing tool names of the latest prefix rebuild, or of the
    /// start record.
    pub tool_names: Vec<String>,
    /// The latest applied change, absent while none has applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective: Option<EffectiveBaseline>,
    /// The highest change generation the runner accepted. `0` before any.
    #[serde(default)]
    pub accepted_generation: u64,
    /// The highest change generation a prefix rebuild or an append applied.
    /// `0` before any.
    #[serde(default)]
    pub applied_generation: u64,
    /// The pending change and its policy, when one is pending.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending: Option<PendingChange>,
    /// The change-policy rungs this session supports, per surface. Absent
    /// when the runner declares none yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rungs: Option<Rungs>,
    /// Session-level capabilities, keyed by name
    /// ([`crate::capabilities::session`]), each declared with `true`.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub session_capabilities: Map<String, Value>,
}

impl Baseline {
    pub fn new(
        items: Vec<BaselineItem>,
        composition_digest: impl Into<String>,
        tool_names: Vec<String>,
    ) -> Self {
        Self {
            items,
            composition_digest: composition_digest.into(),
            tool_names,
            ..Self::default()
        }
    }

    pub fn with_absent_items(mut self, absent_items: Vec<AbsentItem>) -> Self {
        self.absent_items = absent_items;
        self
    }

    pub fn with_effective(mut self, effective: EffectiveBaseline) -> Self {
        self.effective = Some(effective);
        self
    }

    pub fn with_generations(mut self, accepted: u64, applied: u64) -> Self {
        self.accepted_generation = accepted;
        self.applied_generation = applied;
        self
    }

    pub fn with_pending(mut self, pending: PendingChange) -> Self {
        self.pending = Some(pending);
        self
    }

    pub fn with_rungs(mut self, rungs: Rungs) -> Self {
        self.rungs = Some(rungs);
        self
    }

    pub fn with_session_capabilities(mut self, session_capabilities: Map<String, Value>) -> Self {
        self.session_capabilities = session_capabilities;
        self
    }

    /// Whether the session-level capability `name` is declared.
    pub fn declares(&self, name: &str) -> bool {
        self.session_capabilities.get(name) == Some(&Value::Bool(true))
    }
}

/// One fetched plan item's frozen digests.
///
/// Non-exhaustive so later optional members are additive: use
/// [`BaselineItem::new`] and the `with_*` setters, or decode one.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct BaselineItem {
    /// The provider's module id.
    pub provider: String,
    /// Which of the provider's plan items this is, where a provider has more
    /// than one (a tool item and a system-text item, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// The digest the fetch answered for the item.
    pub item_digest: String,
    /// The provider's preflight digest for the item, which the owner
    /// compares at a turn boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_digest: Option<String>,
}

impl BaselineItem {
    pub fn new(provider: impl Into<String>, item_digest: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            item: None,
            item_digest: item_digest.into(),
            preflight_digest: None,
        }
    }

    pub fn with_item(mut self, item: impl Into<String>) -> Self {
        self.item = Some(item.into());
        self
    }

    pub fn with_preflight_digest(mut self, preflight_digest: impl Into<String>) -> Self {
        self.preflight_digest = Some(preflight_digest.into());
        self
    }
}

/// An optional plan item that was not fetched, and why.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct AbsentItem {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// Why the item was not fetched, for example `timeout` or
    /// `unknown_module`. Any other value decodes as a plain string.
    pub reason: String,
}

/// How the latest applied change was applied.
pub mod applied_by {
    /// The prefix was rebuilt from the new manifest.
    pub const FOLD: &str = "fold";
    /// The change was appended mid-conversation, leaving the prefix intact.
    pub const APPEND: &str = "append";
}

/// The latest applied change.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct EffectiveBaseline {
    pub generation: u64,
    /// One of [`applied_by`]; any other value decodes as a plain string.
    pub applied_by: String,
    pub manifest_digest: String,
}

/// A pending change: the highest generation accepted and not yet applied,
/// with its latest policy.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct PendingChange {
    pub generation: u64,
    /// A rung name ([`crate::rungs`]); any other value decodes as a plain
    /// string.
    pub policy: String,
}

/// The rungs a session supports, per surface ([`crate::rungs`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct Rungs {
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub system_text: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{capabilities, errors, rungs, vectors};

    #[test]
    fn baseline_vectors_decode_and_round_trip() {
        let file = vectors::load("baseline.json");
        for case in vectors::cases(&file, "requests") {
            vectors::round_trip::<BaselineRequest>(
                case["name"].as_str().unwrap(),
                &case["request"],
            );
        }
        for case in vectors::cases(&file, "refused_requests") {
            assert!(
                serde_json::from_value::<BaselineRequest>(case["request"].clone()).is_err(),
                "{}",
                case["name"]
            );
        }
        for case in vectors::cases(&file, "answers") {
            let name = case["name"].as_str().unwrap();
            let reply: BaselineReply = vectors::round_trip(name, &case["answer"]);
            assert_eq!(reply.is_not_yet(), reply.baseline.is_none(), "{name}");
            if let Some(baseline) = &reply.baseline {
                assert!(
                    baseline.applied_generation <= baseline.accepted_generation,
                    "{name}"
                );
                for rung in baseline.rungs.iter().flat_map(|r| r.tools.iter()) {
                    assert!(rungs::ALL.contains(&rung.as_str()), "{name}: {rung}");
                }
            }
        }
        for case in vectors::cases(&file, "refusals") {
            assert_eq!(case["refusal"]["code"], errors::SCOPE_OWNER_MISMATCH);
        }
    }

    #[test]
    fn only_true_declares_a_session_capability() {
        let mut caps = Map::new();
        caps.insert(
            capabilities::session::MID_SESSION_APPENDS.into(),
            Value::Bool(true),
        );
        caps.insert(
            capabilities::session::ORDERED_HOOK_PHASES.into(),
            Value::String("true".into()),
        );
        let baseline = Baseline::new(
            vec![BaselineItem::new("aft", "d1").with_preflight_digest("p1")],
            "c1",
            vec!["read".into()],
        )
        .with_session_capabilities(caps);
        assert!(baseline.declares(capabilities::session::MID_SESSION_APPENDS));
        assert!(!baseline.declares(capabilities::session::ORDERED_HOOK_PHASES));
        let reply = BaselineReply::ready(baseline);
        let encoded = serde_json::to_value(&reply).unwrap();
        assert_eq!(
            serde_json::from_value::<BaselineReply>(encoded).unwrap(),
            reply
        );
    }
}
