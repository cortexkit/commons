//! The per-`cutId` step machine.
//!
//! A participant keeps, for each `cutId` it has seen, the step that `cutId`
//! has reached ([`Step`]). Each op that changes state moves it along one
//! edge ([`advance`]); an op that skips a step is refused `invalid_state`
//! naming the current step, and an op that needs earlier state the
//! participant doesn't have is refused `cut_unknown`. The table is
//! CONTRACT.md §8.
//!
//! [`advance`] decides a first request for `(cutId, op)` only. A repeat of
//! a request already answered is decided by idempotency first
//! ([`crate::idempotency`]): it gets the first reply back, or `id_reused`,
//! and never reaches the step machine.
//!
//! After [`advance`] allows an op, [`check_ref_set`] checks its refs: every
//! op after the first one for a `cutId` must name exactly the ref set that
//! `cutId` started with (CONTRACT.md §8.1).

use serde::{Deserialize, Serialize};

use crate::{
    errors::{InvalidStateDetail, RefusalCode},
    refs::Refs,
    Op,
};

/// The step a `cutId` has reached on this participant, as `status` answers
/// it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// No state for this `cutId`.
    None,
    /// Source: `prepare` decided the cut and froze the data.
    Prepared,
    /// Source: `export` admitted every item through engram's session door.
    Exported,
    /// Receiver: `import` staged the cut and checked every digest.
    Imported,
    /// Receiver: `activate` made the staged data live.
    Activated,
    /// Either side: `abort` undid the move here.
    Aborted,
    /// Source: `release` turned the freeze into a tombstone.
    Released,
}

impl Step {
    pub const ALL: [Step; 7] = [
        Step::None,
        Step::Prepared,
        Step::Exported,
        Step::Imported,
        Step::Activated,
        Step::Aborted,
        Step::Released,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Prepared => "prepared",
            Self::Exported => "exported",
            Self::Imported => "imported",
            Self::Activated => "activated",
            Self::Aborted => "aborted",
            Self::Released => "released",
        }
    }
}

/// Why the step machine refused an op.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepRefusal {
    /// `cut_unknown`: the op needs earlier state for this `cutId` and the
    /// participant has none (for example, `export` before `prepare`).
    CutUnknown,
    /// `invalid_state`: the op is out of order; `step` is the current step,
    /// which the refusal's `detail.step` names.
    InvalidState { step: Step },
}

impl StepRefusal {
    pub fn code(self) -> RefusalCode {
        match self {
            Self::CutUnknown => RefusalCode::CutUnknown,
            Self::InvalidState { .. } => RefusalCode::InvalidState,
        }
    }
}

/// The step `op` moves a `cutId` to from `current`, or why it is refused.
/// The reads (`describe`, `busy`, `status`, `diverged`) never change the
/// step and are never refused here.
///
/// - `prepare`: `none` → `prepared` (source).
/// - `export`: `prepared` → `exported` (source).
/// - `import`: `none` → `imported` (receiver).
/// - `activate`: `imported` → `activated` (receiver).
/// - `release`: `exported` → `released` (source).
/// - `abort`: `none`, `prepared`, `exported`, `imported` or `aborted` →
///   `aborted`. Aborting a `cutId` this participant never saw succeeds and
///   touches no data; the step still reads `aborted` afterwards, so a
///   delayed `prepare` or `import` for that `cutId` is refused rather than
///   starting a move entorhinal has given up on.
///
/// `export`, `activate` and `release` from `none` are `cut_unknown`; any
/// other edge not listed is `invalid_state` naming `current`.
pub fn advance(op: Op, current: Step) -> Result<Step, StepRefusal> {
    use Step as S;
    let invalid = Err(StepRefusal::InvalidState { step: current });
    match op {
        Op::Describe | Op::Busy | Op::Status | Op::Diverged => Ok(current),
        Op::Prepare => match current {
            S::None => Ok(S::Prepared),
            _ => invalid,
        },
        Op::Import => match current {
            S::None => Ok(S::Imported),
            _ => invalid,
        },
        Op::Export => match current {
            S::Prepared => Ok(S::Exported),
            S::None => Err(StepRefusal::CutUnknown),
            _ => invalid,
        },
        Op::Activate => match current {
            S::Imported => Ok(S::Activated),
            S::None => Err(StepRefusal::CutUnknown),
            _ => invalid,
        },
        Op::Release => match current {
            S::Exported => Ok(S::Released),
            S::None => Err(StepRefusal::CutUnknown),
            _ => invalid,
        },
        Op::Abort => match current {
            S::None | S::Prepared | S::Exported | S::Imported | S::Aborted => Ok(S::Aborted),
            S::Activated | S::Released => invalid,
        },
    }
}

/// Checks that `refs`, the refs of a request for `op` on a `cutId` at
/// `step`, are the same set as `started_with`, the refs the participant
/// recorded from the first op it accepted for that `cutId`: `prepare` on
/// the source, `import` on the receiver. Call it after [`advance`] has
/// allowed the op.
///
/// It applies to every op after the first: `export`, `release` and the
/// source's `abort` (against `prepare`'s refs), and `activate` and the
/// receiver's `abort` (against `import`'s refs). Order doesn't matter; a
/// different set is refused `invalid_state`, with the current step and the
/// expected refs in the detail. Refs left out would otherwise stay frozen
/// or staged with no op left to clear them.
///
/// It never refuses the first op (`prepare`, `import`), the reads, or an op
/// with no recorded set (`started_with` is `None`): an `abort` of a
/// `cutId` the participant never saw still succeeds and records `aborted`.
pub fn check_ref_set(
    op: Op,
    step: Step,
    started_with: Option<&Refs>,
    refs: &Refs,
) -> Result<(), InvalidStateDetail> {
    let applies = matches!(op, Op::Export | Op::Release | Op::Activate | Op::Abort);
    match started_with {
        Some(expected) if applies && !expected.same_set(refs) => {
            Err(InvalidStateDetail::new(step).with_refs(expected.as_slice().to_vec()))
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::refs::Ref;

    fn refs(names: &[&str]) -> Refs {
        Refs::new(names.iter().map(|n| Ref::new("k", *n)).collect()).unwrap()
    }

    #[test]
    fn later_ops_must_name_the_started_set() {
        let started = refs(&["a", "b"]);
        for (op, step) in [
            (Op::Export, Step::Prepared),
            (Op::Release, Step::Exported),
            (Op::Abort, Step::Prepared),
            (Op::Activate, Step::Imported),
            (Op::Abort, Step::Imported),
        ] {
            assert_eq!(
                check_ref_set(op, step, Some(&started), &refs(&["b", "a"])),
                Ok(()),
                "{op:?}: reordered"
            );
            let refused = check_ref_set(op, step, Some(&started), &refs(&["a"]));
            assert_eq!(
                refused,
                Err(InvalidStateDetail::new(step).with_refs(started.as_slice().to_vec())),
                "{op:?}: subset"
            );
            assert!(
                check_ref_set(op, step, Some(&started), &refs(&["a", "b", "c"])).is_err(),
                "{op:?}: superset"
            );
            assert!(
                check_ref_set(op, step, Some(&started), &refs(&["a", "c"])).is_err(),
                "{op:?}: same size, another set"
            );
        }
    }

    #[test]
    fn first_ops_reads_and_unseen_aborts_are_not_checked() {
        let started = refs(&["a"]);
        let other = refs(&["z"]);
        for op in [Op::Prepare, Op::Import, Op::Status, Op::Diverged, Op::Busy] {
            assert_eq!(
                check_ref_set(op, Step::None, Some(&started), &other),
                Ok(())
            );
        }
        assert_eq!(check_ref_set(Op::Abort, Step::None, None, &other), Ok(()));
        assert_eq!(
            check_ref_set(Op::Abort, Step::Aborted, None, &other),
            Ok(())
        );
    }
}

#[cfg(test)]
mod advance_tests {
    use super::*;

    #[test]
    fn step_spellings_match_serde() {
        for step in Step::ALL {
            assert_eq!(serde_json::to_value(step).unwrap(), step.as_str());
        }
    }

    #[test]
    fn source_and_receiver_paths() {
        assert_eq!(advance(Op::Prepare, Step::None), Ok(Step::Prepared));
        assert_eq!(advance(Op::Export, Step::Prepared), Ok(Step::Exported));
        assert_eq!(advance(Op::Release, Step::Exported), Ok(Step::Released));
        assert_eq!(advance(Op::Import, Step::None), Ok(Step::Imported));
        assert_eq!(advance(Op::Activate, Step::Imported), Ok(Step::Activated));
    }

    #[test]
    fn abort_is_refused_only_after_activated_or_released() {
        for step in Step::ALL {
            let expected = match step {
                Step::Activated | Step::Released => Err(StepRefusal::InvalidState { step }),
                _ => Ok(Step::Aborted),
            };
            assert_eq!(advance(Op::Abort, step), expected, "{step:?}");
        }
    }

    #[test]
    fn skipping_a_step_names_the_current_step() {
        assert_eq!(
            advance(Op::Release, Step::Prepared),
            Err(StepRefusal::InvalidState {
                step: Step::Prepared
            })
        );
        assert_eq!(
            advance(Op::Export, Step::None),
            Err(StepRefusal::CutUnknown)
        );
        assert_eq!(
            advance(Op::Prepare, Step::Aborted),
            Err(StepRefusal::InvalidState {
                step: Step::Aborted
            })
        );
    }

    #[test]
    fn reads_never_change_the_step() {
        for op in [Op::Describe, Op::Busy, Op::Status, Op::Diverged] {
            for step in Step::ALL {
                assert_eq!(advance(op, step), Ok(step));
            }
        }
    }
}
