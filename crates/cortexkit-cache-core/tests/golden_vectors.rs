//! The cross-language parity gate. This drives the cache-core state machine through the
//! frozen golden vectors (`cache-stability-golden-vectors.json`, schema_version 3) — the
//! SAME fixture Magic Context's TS core pins. A green run here means the Rust core branches
//! byte-identically to the reference for every mechanics + durability vector.
//!
//! The fixture is OPAQUE TOKENS, never real hashes: the core only does string-equality +
//! branch. The asserts encode the contract's `common_asserts`:
//!  - every `SOFT+` pass leaves `cached_prefix_bytes` byte-identical to the prior pass;
//!  - every frozen unit replayed on a `SOFT+` pass reproduces its `frozen_payload` verbatim;
//!  - a `SOFT`/`HARD` pass applies exactly `expect_frozen_set_delta` (and only then may bytes
//!    change);
//!  - the anchor is boundary-presence: an absent boundary on a defer pass sets
//!    `reconcile_pending` and KEEPS replaying frozen bytes (never a same-pass rebuild);
//!  - a lineage unit reproduces byte-identical across a `RunStarted` episode boundary.

use std::collections::BTreeMap;

use cortexkit_cache_core::{Action, CoreState, FrozenUnit, PassInput};
use serde::Deserialize;
use serde_json::Value;

const GOLDEN: &str = include_str!("golden/cache-stability-golden-vectors.json");

#[derive(Debug, Deserialize)]
struct GoldenFile {
    schema_version: u32,
    vectors: Vec<Vector>,
}

#[derive(Debug, Deserialize)]
struct Vector {
    name: String,
    #[serde(default)]
    layer: String,
    initial_state: InitialState,
    passes: Vec<Pass>,
}

#[derive(Debug, Deserialize)]
struct InitialState {
    #[serde(default)]
    version: u64,
    boundary_id: String,
    frozen_units: Vec<FrozenUnit>,
    #[serde(default)]
    pending_changes: Vec<FrozenUnit>,
}

#[derive(Debug, Deserialize)]
struct Pass {
    signal: Value,
    boundary_present: String,
    expect_action: Action,
    #[serde(default)]
    new_boundary_id: Option<String>,
    #[serde(default)]
    reconcile_pending: bool,
    #[serde(default)]
    expect_frozen_set_delta: Vec<FrozenUnit>,
    #[serde(default)]
    run_started: bool,
}

/// Translate a fixture pass into the core's typed `PassInput`. This is the SEAM the harness
/// owns in production: it classifies the opaque `signal` into an `Action` (the fixture
/// pre-classifies via `expect_action`, so the gate tests the MECHANICS the classification
/// drives, not the classifier) and supplies the byte-complete rendered units for a bust.
///
/// `pending_keys` is the set of keys currently queued in the core's `pending_changes`. On a
/// HARD, those units MUST be supplied by the core's drain (the coordinator), NOT re-handed in
/// `rendered_units` — so we EXCLUDE them from the rendered set. That makes V6 non-vacuous: a
/// `step_hard` that forgot to drain `pending_changes` would leave the drained unit out of the
/// frozen set and fail the delta assert.
fn pass_to_input(pass: &Pass, pending_keys: &[String]) -> PassInput {
    let run_started = pass.run_started
        || pass
            .signal
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|k| k == "run-started");

    let mut input = PassInput {
        proposed: Some(pass.expect_action),
        boundary_present: pass.boundary_present.clone(),
        new_boundary_id: pass.new_boundary_id.clone(),
        run_started,
        ..Default::default()
    };

    match pass.expect_action {
        Action::SoftPlus => {
            // A `drop-queued` defer routes the queued unit into pending_changes (the
            // observer-half drop accumulates while the prefix replays frozen). The fixture
            // keeps the queued bytes in the LATER HARD's delta, so mirror them here.
            if pass
                .signal
                .get("kind")
                .and_then(Value::as_str)
                .is_some_and(|k| k == "drop-queued")
            {
                input.queued = vec![FrozenUnit {
                    key: "drop1".into(),
                    kind: "drop".into(),
                    frozen_payload: "[dropped 1]".into(),
                    durability_class: cortexkit_cache_core::DurabilityClass::Lineage,
                    reset_rule: "survive + advance-only-merge, never reset".into(),
                }];
            }
        }
        Action::Soft => {
            input.rendered_units = pass.expect_frozen_set_delta.clone();
        }
        Action::Hard => {
            // The newly rendered baseline = the delta MINUS whatever the core will drain from
            // pending_changes. Withholding the drained unit forces the core to supply it.
            input.rendered_units = pass
                .expect_frozen_set_delta
                .iter()
                .filter(|u| !pending_keys.contains(&u.key))
                .cloned()
                .collect();
        }
    }
    input
}

#[test]
fn golden_fixture_is_schema_v3_with_eleven_vectors() {
    let file: GoldenFile = serde_json::from_str(GOLDEN).expect("golden fixture parses");
    assert_eq!(file.schema_version, 3, "pinned to schema_version 3");
    assert_eq!(
        file.vectors.len(),
        11,
        "11 vectors (8 mechanics + V9 durability + V10 coverage-extending SOFT + V11 never-minted boundary)"
    );
}

#[test]
fn all_golden_vectors_pass() {
    let file: GoldenFile = serde_json::from_str(GOLDEN).expect("golden fixture parses");

    for vector in &file.vectors {
        run_vector(vector);
    }
}

fn run_vector(vector: &Vector) {
    let mut state = CoreState {
        version: vector.initial_state.version,
        boundary_id: vector.initial_state.boundary_id.clone(),
        frozen_units: vector.initial_state.frozen_units.clone(),
        pending_changes: vector.initial_state.pending_changes.clone(),
        reconcile_pending: false,
    };

    // Per-unit replay map: key -> the frozen_payload that must reproduce verbatim on defer.
    let mut frozen_seen: BTreeMap<String, String> = state
        .frozen_units
        .iter()
        .map(|u| (u.key.clone(), u.frozen_payload.clone()))
        .collect();

    let mut prev_bytes = state.cached_prefix_bytes();

    for (i, pass) in vector.passes.iter().enumerate() {
        let pending_keys: Vec<String> = state
            .pending_changes
            .iter()
            .map(|u| u.key.clone())
            .collect();
        let input = pass_to_input(pass, &pending_keys);
        let before_bytes = state.cached_prefix_bytes();
        let result = state.step(input);

        // 1. The executed action matches the fixture.
        assert_eq!(
            result.action, pass.expect_action,
            "{}: pass {i} action mismatch",
            vector.name
        );

        let after_bytes = state.cached_prefix_bytes();

        match pass.expect_action {
            Action::SoftPlus => {
                // 2. A defer recomputes reconcile_pending from boundary-presence,
                //    BIDIRECTIONALLY: boundary present => not pending; absent => pending. This
                //    is what makes V10 non-vacuous — its SOFT+ passes at the NEW anchor b1 must
                //    read NOT-pending, which only holds if the prior SOFT actually advanced the
                //    anchor b0->b1 (else b1 would be absent => spurious reconcile).
                assert_eq!(
                    result.reconcile_pending, pass.reconcile_pending,
                    "{}: pass {i} SOFT+ reconcile_pending must equal boundary-absence",
                    vector.name
                );
                // 3. A defer pass NEVER changes cached_prefix_bytes (the byte-stability
                //    invariant), even when the boundary is absent (revert) — it keeps
                //    replaying the frozen bytes.
                assert_eq!(
                    after_bytes, before_bytes,
                    "{}: pass {i} SOFT+ must not change cached_prefix_bytes",
                    vector.name
                );
                // 4. Every previously-frozen unit still reproduces its frozen_payload
                //    verbatim (no re-derivation on defer).
                for unit in &state.frozen_units {
                    if let Some(expected) = frozen_seen.get(&unit.key) {
                        assert_eq!(
                            &unit.frozen_payload, expected,
                            "{}: pass {i} frozen unit '{}' re-derived on defer",
                            vector.name, unit.key
                        );
                    }
                }
            }
            Action::Soft | Action::Hard => {
                // 5. A bust applies EXACTLY the expected frozen-set delta (replace-by-key /
                //    append), and the resulting frozen set carries each delta unit verbatim.
                for delta in &pass.expect_frozen_set_delta {
                    let stored = state
                        .frozen_units
                        .iter()
                        .find(|u| u.key == delta.key)
                        .unwrap_or_else(|| {
                            panic!(
                                "{}: pass {i} bust delta '{}' not in frozen set",
                                vector.name, delta.key
                            )
                        });
                    assert_eq!(
                        stored, delta,
                        "{}: pass {i} bust unit '{}' bytes diverge from fixture",
                        vector.name, delta.key
                    );
                    frozen_seen.insert(delta.key.clone(), delta.frozen_payload.clone());
                }
                // 6. A bust carrying new_boundary_id advances the coverage anchor — HARD
                //    always; SOFT only as a (guarded) coverage-extending bust (V10). Asserting
                //    this for the SOFT case is the load-bearing V10 check: the anchor moved on a
                //    SOFT while m0 bytes stayed frozen (the delta assert above proves m0 frozen).
                if let Some(b) = &pass.new_boundary_id {
                    assert_eq!(
                        &state.boundary_id, b,
                        "{}: pass {i} bust must advance the boundary to the fixture id",
                        vector.name
                    );
                }
                // 7. A HARD drains all deferred work (pending_changes empty after).
                if pass.expect_action == Action::Hard {
                    assert!(
                        state.pending_changes.is_empty(),
                        "{}: pass {i} HARD must drain deferred work",
                        vector.name
                    );
                }
            }
        }

        prev_bytes = after_bytes;
    }

    let _ = prev_bytes;
}

/// V9 specifically: a RunStarted/episode boundary reproduces lineage units byte-identical
/// (the class that bit the identity-lead — green within-run, busts at the episode boundary).
#[test]
fn cross_episode_lineage_reproduces_byte_identical() {
    let file: GoldenFile = serde_json::from_str(GOLDEN).expect("golden fixture parses");
    let v9 = file
        .vectors
        .iter()
        .find(|v| v.name.starts_with("V9"))
        .expect("V9 present");
    assert_eq!(v9.layer, "durability");

    let mut state = CoreState {
        version: v9.initial_state.version,
        boundary_id: v9.initial_state.boundary_id.clone(),
        frozen_units: v9.initial_state.frozen_units.clone(),
        pending_changes: v9.initial_state.pending_changes.clone(),
        reconcile_pending: false,
    };

    let pre_episode = state.cached_prefix_bytes();
    for pass in &v9.passes {
        let run_started = pass
            .signal
            .get("kind")
            .and_then(Value::as_str)
            .is_some_and(|k| k == "run-started");
        let pending_keys: Vec<String> = state
            .pending_changes
            .iter()
            .map(|u| u.key.clone())
            .collect();
        let before = state.cached_prefix_bytes();
        state.step(pass_to_input(pass, &pending_keys));
        if run_started {
            assert_eq!(
                state.cached_prefix_bytes(),
                before,
                "RunStarted must not bust the cached prefix (lineage byte-identical)"
            );
        }
    }
    // The whole lineage reproduced byte-identical across the episode boundary.
    assert_eq!(
        state.cached_prefix_bytes(),
        pre_episode,
        "lineage units must reproduce byte-identical across the episode boundary"
    );
}
