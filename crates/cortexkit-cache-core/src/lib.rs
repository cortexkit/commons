//! Harness-neutral prompt-cache-stability core.
//!
//! This is the shared per-pass state machine that lets a harness pin a render-config
//! across turns without thrashing the provider prefix cache. It is consumed by the
//! Magic Context observer harness and the llm-runner author harness; both pin the same
//! golden-vector contract (`cache-stability-golden-vectors.json`, schema_version 2).
//!
//! ## The one invariant
//!
//! The harness renders each affected unit to **final bytes on a cache-busting pass**
//! ([`Action::Soft`] / [`Action::Hard`]) and the core **freezes those bytes**. A defer
//! pass ([`Action::SoftPlus`]) **places the frozen bytes verbatim** — there is no render
//! call on a defer pass, so defer-pass re-derivation is structurally impossible. The core
//! never interprets a `frozen_payload`; it only stores and replays it.
//!
//! ## The cache anchor is BOUNDARY-PRESENCE
//!
//! The covered prefix is REPLACED by the frozen bytes; the only per-pass validity check is
//! whether the boundary id is still present in the live array (`boundary_present ==
//! state.boundary_id`). There is NO content fingerprint over the covered prefix, so no
//! collision surface: an in-prefix edit is summarized away (intentional lossiness, not a
//! stale cache), and a revert that removes the boundary keeps replaying the frozen bytes
//! this pass (reconcile_pending) and reconciles on the next cache-busting pass.
//!
//! ## Division of responsibility
//!
//! Trigger causes are a **harness-provided predicate**, never baked into the core: the
//! harness classifies each signal into an [`Action`] and the core applies the
//! freeze/replay/coordinator/durability mechanics for that class. The core owns the
//! boundary-presence branch, the byte-complete freeze-on-bust / replay-on-defer discipline,
//! the deferred-work coordinator (a HARD bust from any cause drains all deferred work), the
//! `durability_class` reset rule across episode boundaries, and the version stamp for the
//! harness's CAS write-back.

use serde::{Deserialize, Serialize};

/// The classifier verdict for one pass. The HARNESS assigns this (its trigger predicate);
/// the core applies the mechanics. `SoftPlus` = defer (replay frozen bytes verbatim, no
/// render). `Soft` = bust at the volatile-delta breakpoint (the stable baseline stays
/// cached). `Hard` = the whole prefix rebuilds into a new frozen baseline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Action {
    #[serde(rename = "SOFT+")]
    SoftPlus,
    #[serde(rename = "SOFT")]
    Soft,
    #[serde(rename = "HARD")]
    Hard,
}

/// Whether a frozen unit survives an episode (`RunStarted`) boundary.
///
/// Every CACHE frozen unit is `Lineage` today: a unit compacts the conversation prefix,
/// which is lineage-cumulative (a drop in episode 1 stays dropped in episode 5; it does not
/// un-compact at a new run). `Episode` is RESERVED for schema-completeness — the per-episode
/// reset state (run_config/usage/completed_steps) is WAL replay-state, a separate structure,
/// not a cache frozen unit — so a future run-scoped frozen unit is expressible without a
/// schema break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DurabilityClass {
    Episode,
    Lineage,
}

/// A frozen render unit: an opaque byte-complete payload the harness rendered on a bust
/// pass. The core stores and replays `frozen_payload` verbatim and NEVER interprets it
/// (kind is opaque too — `drop` | `strip` | `skeleton` | `synthesized-region` | `injection`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrozenUnit {
    pub key: String,
    pub kind: String,
    /// EXACT emitted bytes as of the freeze (bust) pass — NOT inputs to re-render. A defer
    /// pass replays these verbatim; the core never re-derives them.
    pub frozen_payload: String,
    pub durability_class: DurabilityClass,
    #[serde(default)]
    pub reset_rule: String,
}

/// The core's durable per-pass state. One atomic value: the harness CAS-writes it back
/// whole (units + boundary + version), never per-field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CoreState {
    pub version: u64,
    /// The coverage descriptor: the id the covered prefix is spliced out at. A CAS-retry
    /// re-splices the SAME id. Compared for equality against `boundary_present`.
    pub boundary_id: String,
    pub frozen_units: Vec<FrozenUnit>,
    /// Deferred units queued on a `SoftPlus` pass (drop-queued); drained into the next bust.
    #[serde(default)]
    pub pending_changes: Vec<FrozenUnit>,
    /// Set when a defer pass lost the boundary (a revert removed it). The harness reads this
    /// to decide that the NEXT cache-busting pass rematerializes against the live array.
    #[serde(default)]
    pub reconcile_pending: bool,
}

/// One pass into the core. `proposed` is the harness's classification; `boundary_present`
/// is the opaque live-boundary token; `rendered_units` are the byte-complete units the
/// harness rendered for a bust (empty on `SoftPlus`).
#[derive(Debug, Clone, Default)]
pub struct PassInput {
    pub proposed: Option<Action>,
    pub boundary_present: String,
    /// Byte-complete units the harness rendered on a `Soft`/`Hard` bust. Frozen into state.
    pub rendered_units: Vec<FrozenUnit>,
    /// Units to queue into `pending_changes` on a `SoftPlus` defer (drop-queued). They wait
    /// for the next bust.
    pub queued: Vec<FrozenUnit>,
    /// The rematerialized boundary id minted on a `Hard` fold (or a bust that rematerializes
    /// after a boundary loss).
    pub new_boundary_id: Option<String>,
    /// A `RunStarted` / episode boundary: lineage units reproduce byte-identical, episode
    /// units reset.
    pub run_started: bool,
}

impl PassInput {
    /// Construct a pass input with an explicit action and boundary token.
    pub fn new(proposed: Action, boundary_present: impl Into<String>) -> Self {
        PassInput {
            proposed: Some(proposed),
            boundary_present: boundary_present.into(),
            ..Default::default()
        }
    }
}

/// The result of a pass: the executed action and whether reconciliation is now pending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepResult {
    pub action: Action,
    pub reconcile_pending: bool,
}

impl CoreState {
    /// The cached-prefix bytes: the in-order concatenation of every frozen unit's payload.
    /// Two consecutive `SoftPlus` passes MUST produce an identical value (the byte-stability
    /// invariant) — the core proves it structurally by not mutating frozen units on a defer.
    pub fn cached_prefix_bytes(&self) -> String {
        self.frozen_units
            .iter()
            .map(|u| u.frozen_payload.as_str())
            .collect()
    }

    /// Apply one pass. Returns the executed [`StepResult`] and mutates `self` in place
    /// (the harness then CAS-writes the whole value back).
    ///
    /// # Panics
    /// Panics if `input.proposed` is `None` (the harness MUST classify the signal before
    /// stepping — trigger causes are harness-provided).
    pub fn step(&mut self, input: PassInput) -> StepResult {
        let proposed = input
            .proposed
            .expect("harness must classify the signal into an Action before stepping");
        let boundary_match = input.boundary_present == self.boundary_id;

        match proposed {
            Action::SoftPlus => self.step_defer(input, boundary_match),
            Action::Soft => self.step_soft(input, boundary_match),
            Action::Hard => self.step_hard(input),
        }
    }

    /// Defer: no render call, replay frozen bytes verbatim. Queue any drop-queued units into
    /// `pending_changes`. If the boundary is absent (a revert removed it), keep replaying the
    /// frozen bytes this pass and mark `reconcile_pending` — NEVER a blind same-pass rebuild.
    /// On a `RunStarted` boundary, lineage units carry forward byte-identical and episode
    /// units reset.
    fn step_defer(&mut self, input: PassInput, boundary_match: bool) -> StepResult {
        for unit in input.queued {
            self.pending_changes.push(unit);
        }

        if input.run_started {
            // Lineage units survive byte-identical (their frozen_payload is untouched);
            // episode units reset at the run boundary. The cache set is all-lineage today,
            // so this is a no-op in practice, but the reset rule is enforced structurally.
            self.frozen_units
                .retain(|u| u.durability_class == DurabilityClass::Lineage);
        }

        // boundary_match => the covered prefix splices/replaces and frozen bytes replay.
        // boundary absent => reuse this pass, reconcile on the next bust.
        self.reconcile_pending = !boundary_match;

        StepResult {
            action: Action::SoftPlus,
            reconcile_pending: self.reconcile_pending,
        }
    }

    /// Soft bust: the volatile delta re-renders, the stable baseline (m0 frozen bytes) stays
    /// frozen. Freeze the rendered delta units (replace same-key, append new).
    ///
    /// `boundary_id` is the COVERAGE anchor (the last raw item any summary covers, m0 OR the
    /// delta). A SOFT MAY advance it when `new_boundary_id` is `Some` — used when the volatile
    /// delta now summarizes content past the prior anchor (e.g. a new compartment rides the
    /// delta and extends coverage over raw tail items). The m0 frozen bytes are NEVER mutated
    /// on a SOFT; only the coverage anchor moves, so the byte-stability invariant holds. `None`
    /// leaves the boundary unchanged (the common case: a delta that rides within existing
    /// coverage, e.g. a memory delta). `reconcile_pending` is untouched — a pending reconcile
    /// (m0 itself stale) is cleared only by a HARD rematerialize, never a SOFT.
    ///
    /// The anchor advance is GUARDED: it applies only when the prior anchor is present
    /// (`boundary_match`) AND no reconcile is pending. A coverage-extending SOFT is only
    /// coherent when the current anchor is live and m0 is not already stale — advancing the
    /// anchor while a reconcile is pending would strand the stale m0 under a fresh anchor (the
    /// next defer would clear `reconcile_pending` against the new anchor and the needed HARD
    /// rematerialize would never fire). Under a correct harness classifier this never arises
    /// (`reconcile_pending` routes Defer or HARD, never a coverage-extending SOFT), but this is
    /// a shared cache-stability primitive, so the guard is enforced in the core, not assumed.
    fn step_soft(&mut self, input: PassInput, boundary_match: bool) -> StepResult {
        self.apply_units(input.rendered_units);
        if boundary_match && !self.reconcile_pending {
            if let Some(new_boundary) = input.new_boundary_id {
                self.boundary_id = new_boundary;
            }
        }
        self.version += 1;
        StepResult {
            action: Action::Soft,
            reconcile_pending: self.reconcile_pending,
        }
    }

    /// Hard bust: the whole prefix rebuilds into a new frozen baseline. Freeze the rendered
    /// units AND drain ALL deferred work into this one bust (a HARD from any cause drains the
    /// coordinator). Mint the new boundary id; clear `reconcile_pending` (the rematerialize
    /// reconciles any earlier boundary loss).
    fn step_hard(&mut self, input: PassInput) -> StepResult {
        let mut units = input.rendered_units;
        units.append(&mut self.pending_changes);
        self.apply_units(units);
        if let Some(new_boundary) = input.new_boundary_id {
            self.boundary_id = new_boundary;
        }
        self.reconcile_pending = false;
        self.version += 1;
        StepResult {
            action: Action::Hard,
            reconcile_pending: false,
        }
    }

    /// Freeze a set of rendered units into the frozen set: a unit with an existing key
    /// REPLACES it (a re-materialized region), a new key appends. Order is preserved (existing
    /// keys keep their slot; new keys append in input order) — the cached-prefix byte order
    /// is load-bearing.
    fn apply_units(&mut self, units: Vec<FrozenUnit>) {
        for unit in units {
            if let Some(slot) = self.frozen_units.iter_mut().find(|u| u.key == unit.key) {
                *slot = unit;
            } else {
                self.frozen_units.push(unit);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Mechanism proofs with HAND-WRITTEN expectations (independent of the golden fixture).
    //! The golden harness feeds `expect_action` as the proposed action, so its action-equality
    //! assert is near-tautological; these pin the load-bearing mechanics non-vacuously: a
    //! broken `step_defer`/`step_hard` fails HERE even if the golden harness let it slide.

    use super::*;

    fn unit(key: &str, payload: &str, class: DurabilityClass) -> FrozenUnit {
        FrozenUnit {
            key: key.into(),
            kind: "synthesized-region".into(),
            frozen_payload: payload.into(),
            durability_class: class,
            reset_rule: String::new(),
        }
    }

    fn state_with(units: Vec<FrozenUnit>, boundary: &str) -> CoreState {
        CoreState {
            version: 0,
            boundary_id: boundary.into(),
            frozen_units: units,
            pending_changes: vec![],
            reconcile_pending: false,
        }
    }

    #[test]
    fn defer_does_not_mutate_frozen_bytes_or_render() {
        let mut state = state_with(
            vec![
                unit("m0", "<h>BASE</h>", DurabilityClass::Lineage),
                unit("m1", "(empty)", DurabilityClass::Lineage),
            ],
            "b0",
        );
        let before = state.cached_prefix_bytes();
        // A defer pass carries NO rendered_units; the core must replay verbatim.
        let r = state.step(PassInput::new(Action::SoftPlus, "b0"));
        assert_eq!(r.action, Action::SoftPlus);
        assert!(!r.reconcile_pending);
        assert_eq!(state.cached_prefix_bytes(), before, "defer changed bytes");
        assert_eq!(
            state.version, 0,
            "defer must not bump version (no CAS write needed)"
        );
    }

    #[test]
    fn defer_boundary_absent_keeps_bytes_and_sets_reconcile_pending() {
        let mut state = state_with(
            vec![unit("m0", "<h>BASE</h>", DurabilityClass::Lineage)],
            "b0",
        );
        let before = state.cached_prefix_bytes();
        // Boundary removed by a revert: '-' != 'b0'. The core must NOT rebuild this pass.
        let r = state.step(PassInput::new(Action::SoftPlus, "-"));
        assert_eq!(r.action, Action::SoftPlus, "revert pass must not bust");
        assert!(r.reconcile_pending, "boundary absent must flag reconcile");
        assert_eq!(
            state.cached_prefix_bytes(),
            before,
            "revert must keep frozen bytes"
        );
    }

    #[test]
    fn hard_drains_pending_changes_into_the_bust() {
        let mut state = state_with(
            vec![unit("m0", "<h>BASE</h>", DurabilityClass::Lineage)],
            "b0",
        );
        // SOFT+ with a drop queued: accumulates in pending_changes, no bust.
        let mut defer = PassInput::new(Action::SoftPlus, "b0");
        defer.queued = vec![unit("drop1", "[dropped 1]", DurabilityClass::Lineage)];
        state.step(defer);
        assert_eq!(state.pending_changes.len(), 1, "drop must queue");
        assert!(
            !state.cached_prefix_bytes().contains("[dropped 1]"),
            "queued drop must NOT appear in the prefix until the bust drains it",
        );

        // HARD: rendered baseline does NOT include the drop; the core must DRAIN it.
        let mut hard = PassInput::new(Action::Hard, "b0");
        hard.rendered_units = vec![unit("m0", "<h>FOLDED</h>", DurabilityClass::Lineage)];
        hard.new_boundary_id = Some("b1".into());
        let r = state.step(hard);

        assert_eq!(r.action, Action::Hard);
        assert!(
            state.pending_changes.is_empty(),
            "HARD must drain deferred work"
        );
        assert_eq!(state.boundary_id, "b1", "HARD mints the new boundary");
        assert!(
            state.cached_prefix_bytes().contains("[dropped 1]"),
            "drained drop must now be in the frozen prefix",
        );
        assert!(!r.reconcile_pending, "HARD clears reconcile");
    }

    #[test]
    fn soft_replaces_by_key_keeps_slot_appends_new() {
        let mut state = state_with(
            vec![
                unit("m0", "<h>BASE</h>", DurabilityClass::Lineage),
                unit("m1", "(empty)", DurabilityClass::Lineage),
            ],
            "b0",
        );
        // SOFT re-renders m1 in place and appends a new delta; m0 stays frozen + keeps slot 0.
        let mut soft = PassInput::new(Action::Soft, "b0");
        soft.rendered_units = vec![
            unit("m1", "<delta>X</delta>", DurabilityClass::Lineage),
            unit("d1", "<add>Y</add>", DurabilityClass::Lineage),
        ];
        state.step(soft);
        let keys: Vec<&str> = state.frozen_units.iter().map(|u| u.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["m0", "m1", "d1"],
            "replace keeps slot, new appends"
        );
        assert_eq!(
            state.frozen_units[0].frozen_payload, "<h>BASE</h>",
            "m0 stays frozen"
        );
        assert_eq!(
            state.frozen_units[1].frozen_payload, "<delta>X</delta>",
            "m1 replaced"
        );
        assert_eq!(
            state.boundary_id, "b0",
            "SOFT with new_boundary_id=None leaves the anchor unchanged (the common case + the \
             llm-runner consumer path)"
        );
    }

    #[test]
    fn coverage_extending_soft_advances_anchor_keeps_m0_frozen() {
        // m0 covers up to b0; m1 is the volatile delta slot.
        let mut state = state_with(
            vec![
                unit("m0", "<h>BASE</h>", DurabilityClass::Lineage),
                unit("m1", "(empty)", DurabilityClass::Lineage),
            ],
            "b0",
        );
        let m0_before = state.frozen_units[0].frozen_payload.clone();

        // A SOFT whose delta now summarizes a new compartment that extends coverage past b0 to
        // b1 (the m1-takes-a-compartment case): re-render m1 AND advance the coverage anchor.
        let mut soft = PassInput::new(Action::Soft, "b0");
        soft.rendered_units = vec![unit(
            "m1",
            "<compartment>C1</compartment>",
            DurabilityClass::Lineage,
        )];
        soft.new_boundary_id = Some("b1".into());
        let r = state.step(soft);

        assert_eq!(r.action, Action::Soft);
        assert_eq!(
            state.boundary_id, "b1",
            "a coverage-extending SOFT advances the anchor to the new coverage end"
        );
        assert_eq!(
            state.frozen_units[0].frozen_payload, m0_before,
            "m0 frozen bytes must NOT change on the coverage-extending SOFT (only the anchor moves)"
        );

        // A defer at the NEW anchor replays byte-identical, no reconcile.
        let before = state.cached_prefix_bytes();
        let r = state.step(PassInput::new(Action::SoftPlus, "b1"));
        assert_eq!(r.action, Action::SoftPlus);
        assert!(
            !r.reconcile_pending,
            "defer at the new anchor must not reconcile"
        );
        assert_eq!(
            state.cached_prefix_bytes(),
            before,
            "defer replays m0+m1 byte-identical after the coverage-extending SOFT"
        );

        // A revert that removes the new boundary from the live array -> reconcile (the anchor
        // moved to b1, so a revert below b1 makes b1 absent).
        let r = state.step(PassInput::new(Action::SoftPlus, "-"));
        assert!(
            r.reconcile_pending,
            "a revert below the new anchor must flag reconcile"
        );

        // The reconcile-forced HARD rematerializes m0 against the live array and re-mints the
        // anchor: m0 bytes replaced, new boundary, reconcile cleared.
        let mut hard = PassInput::new(Action::Hard, "-");
        hard.rendered_units = vec![unit("m0", "<h>REMAT</h>", DurabilityClass::Lineage)];
        hard.new_boundary_id = Some("b2".into());
        let r = state.step(hard);
        assert_eq!(r.action, Action::Hard);
        assert!(!r.reconcile_pending, "HARD clears the pending reconcile");
        assert_eq!(state.boundary_id, "b2", "HARD re-mints the anchor");
        assert_eq!(
            state.frozen_units[0].frozen_payload, "<h>REMAT</h>",
            "HARD rematerializes m0"
        );
    }

    #[test]
    fn soft_does_not_advance_anchor_while_reconcile_pending() {
        // The adversarial case: a misclassified (or future-consumer) coverage-extending SOFT
        // arriving while m0 is already stale (reconcile_pending) must NOT move the anchor — else
        // the stale m0 gets stranded under a fresh anchor and the needed HARD never fires.
        let mut state = state_with(
            vec![
                unit("m0", "<h>OLD_BASE</h>", DurabilityClass::Lineage),
                unit("m1", "(empty)", DurabilityClass::Lineage),
            ],
            "b0",
        );

        // A revert removes b0 -> defer reuses this pass and sets reconcile_pending.
        let r = state.step(PassInput::new(Action::SoftPlus, "-"));
        assert!(r.reconcile_pending, "revert must flag reconcile");

        // An erroneous coverage-extending SOFT while reconcile is pending: the anchor must hold.
        let mut soft = PassInput::new(Action::Soft, "-");
        soft.rendered_units = vec![unit(
            "m1",
            "<compartment>C1</compartment>",
            DurabilityClass::Lineage,
        )];
        soft.new_boundary_id = Some("b1".into());
        state.step(soft);
        assert_eq!(
            state.boundary_id, "b0",
            "anchor must NOT advance on a SOFT while reconcile is pending (no stranded stale m0)"
        );

        // The next absent pass still forces reconcile against the original anchor (not stranded).
        let r = state.step(PassInput::new(Action::SoftPlus, "-"));
        assert!(
            r.reconcile_pending,
            "the needed reconcile must still fire — it was not stranded under a moved anchor"
        );
    }

    #[test]
    fn run_started_keeps_lineage_resets_episode() {
        let mut state = state_with(
            vec![
                unit("m0", "<h>BASE</h>", DurabilityClass::Lineage),
                unit("ep", "run-scoped", DurabilityClass::Episode),
            ],
            "b0",
        );
        let mut run = PassInput::new(Action::SoftPlus, "b0");
        run.run_started = true;
        state.step(run);
        let keys: Vec<&str> = state.frozen_units.iter().map(|u| u.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["m0"],
            "episode unit resets at RunStarted, lineage survives"
        );
        assert_eq!(
            state.frozen_units[0].frozen_payload, "<h>BASE</h>",
            "lineage byte-identical"
        );
    }

    #[test]
    #[should_panic(expected = "must classify")]
    fn step_panics_if_signal_unclassified() {
        let mut state = state_with(vec![unit("m0", "x", DurabilityClass::Lineage)], "b0");
        state.step(PassInput {
            boundary_present: "b0".into(),
            ..Default::default()
        });
    }
}
