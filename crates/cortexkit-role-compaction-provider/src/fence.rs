//! The request fence and the version rule: whether the runner acts on an
//! answer.
//!
//! Before each call the runner durably records the id of the request it is
//! issuing, its newest. An answer is acted on only if it names that newest
//! request and arrives before that request's call deadline. A
//! CompactionMessage must also carry a version higher than the last applied
//! one, be well formed, and end no later than the newest message the
//! request's status reported. Anything else is recorded and never applied,
//! whatever its version, and the runner keeps serving the last applied
//! CompactionMessage. The provider's answer to the next request carries the
//! content instead.
//!
//! The fence covers every answer, not only CompactionMessages: a late
//! `refuse` must not end a run that has moved on, and a late `noop` must not
//! move the cursor.

use crate::answer::{MessageProblem, StepAnswer};

/// What the runner knows when an answer arrives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FenceState {
    /// The id of the newest request the runner issued for this session.
    pub newest_request_id: String,
    /// The version of the last applied CompactionMessage; `None` before
    /// Setup's initial CompactionMessage is recorded.
    pub last_applied_version: Option<u64>,
    /// The newest ordinal the newest request's status reported; `None` when
    /// the lineage had no message yet.
    pub newest_ordinal: Option<u64>,
    /// The newest request's call deadline, in milliseconds since the Unix
    /// epoch on the runner's clock: the instant the runner times the call
    /// out and goes on with the last applied CompactionMessage. `None` when
    /// the runner set no deadline for the call.
    pub newest_deadline_ms: Option<u64>,
}

/// Why an answer is recorded but not acted on. The names are the values a
/// later status carries in `last_not_applied.reason`; a runner's own
/// structural checks add `structural` beside them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotApplied {
    /// The answer names a request other than the newest issued.
    SupersededRequest,
    /// The answer names the newest request but arrived at or after that
    /// request's call deadline, when the runner had already timed the call
    /// out.
    Late,
    /// The CompactionMessage's version is not higher than the last applied.
    StaleVersion,
    /// The CompactionMessage is malformed on its own.
    Malformed(MessageProblem),
    /// The range ends after the newest message the request reported.
    RangeBeyondNewest,
}

impl NotApplied {
    /// The reason's name as the vectors and `last_not_applied.reason` spell
    /// it.
    pub fn name(self) -> &'static str {
        match self {
            Self::SupersededRequest => "superseded_request",
            Self::Late => "late",
            Self::StaleVersion => "stale_version",
            Self::Malformed(problem) => problem.name(),
            Self::RangeBeyondNewest => "range_beyond_newest",
        }
    }
}

/// The runner's decision for one answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Disposition {
    /// Act on it: apply the CompactionMessage, record the `noop`, enter the
    /// wait, or end the run for the `refuse`.
    Act,
    /// Record it, act on nothing, keep the last applied CompactionMessage.
    NotApplied(NotApplied),
}

impl Disposition {
    /// The decision's name as the vectors spell it: `act`, or the reason.
    pub fn name(self) -> &'static str {
        match self {
            Self::Act => "act",
            Self::NotApplied(reason) => reason.name(),
        }
    }
}

/// Decide whether the runner acts on `answer`, which arrived at
/// `arrived_at_ms` on the runner's clock. Checks run in a fixed order: the
/// request fence, then the call deadline, then the message on its own, then
/// the version, then the range against the newest message.
pub fn dispose(state: &FenceState, answer: &StepAnswer, arrived_at_ms: u64) -> Disposition {
    if answer.request_id() != state.newest_request_id {
        return Disposition::NotApplied(NotApplied::SupersededRequest);
    }
    if state
        .newest_deadline_ms
        .is_some_and(|deadline| arrived_at_ms >= deadline)
    {
        return Disposition::NotApplied(NotApplied::Late);
    }
    let StepAnswer::CompactionMessage { compaction, .. } = answer else {
        return Disposition::Act;
    };
    if let Err(problem) = compaction.check() {
        return Disposition::NotApplied(NotApplied::Malformed(problem));
    }
    if let Some(last) = state.last_applied_version {
        if compaction.version <= last {
            return Disposition::NotApplied(NotApplied::StaleVersion);
        }
    }
    // The first ordinal after the newest message; 0 when there is none, so
    // only an empty range at 0 fits an empty lineage.
    let end = state.newest_ordinal.map_or(0, |newest| newest + 1);
    if compaction.range.to > end {
        return Disposition::NotApplied(NotApplied::RangeBeyondNewest);
    }
    Disposition::Act
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vectors;

    #[test]
    fn fence_vectors_dispose_as_recorded() {
        let file = vectors::load("fence.json");
        for case in vectors::cases(&file, "cases") {
            let name = case["name"].as_str().unwrap();
            let state = &case["state"];
            let state = FenceState {
                newest_request_id: state["newest_request_id"].as_str().unwrap().to_owned(),
                last_applied_version: state["last_applied_version"].as_u64(),
                newest_ordinal: state["newest_ordinal"].as_u64(),
                newest_deadline_ms: state["newest_deadline_ms"].as_u64(),
            };
            // A case without a deadline has no arrival time either: the
            // deadline check cannot fire for it.
            assert_eq!(
                state.newest_deadline_ms.is_some(),
                case.get("arrived_at_ms").is_some(),
                "{name}: a deadline and an arrival time come together"
            );
            let arrived_at_ms = case["arrived_at_ms"].as_u64().unwrap_or(0);
            let answer: StepAnswer = serde_json::from_value(case["answer"].clone())
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(
                dispose(&state, &answer, arrived_at_ms).name(),
                case["disposition"].as_str().unwrap(),
                "{name}"
            );
        }
    }

    #[test]
    fn a_delayed_answer_never_applies_over_a_fresh_one() {
        // The runner issued r1, got no answer in time, and issued r2. The
        // provider (restarted in between, so its counter started lower)
        // answered r2 at version 5, which the runner applied. The answer to
        // r1, built before the restart, then arrives with version 9: its
        // higher version must not let it replace newer content.
        let after_fresh = FenceState {
            newest_request_id: "r2".into(),
            last_applied_version: Some(5),
            newest_ordinal: Some(40),
            newest_deadline_ms: None,
        };
        let delayed: StepAnswer = serde_json::from_value(serde_json::json!({
            "answer": "compaction_message",
            "request_id": "r1",
            "compaction": {"compaction_id": "c-old", "version": 9,
                           "range": {"from": 0, "to": 30}, "replacement": []}
        }))
        .unwrap();
        assert_eq!(
            dispose(&after_fresh, &delayed, 0),
            Disposition::NotApplied(NotApplied::SupersededRequest)
        );
    }

    #[test]
    fn an_answer_to_the_newest_request_after_its_deadline_never_applies() {
        // The runner issued r2 with a deadline at t=2000, timed the call out
        // and went on with the last applied version 5, issuing nothing newer.
        // The answer to r2 then arrives at t=2600 with a higher version: it
        // still names the newest request, but it is late.
        let timed_out = FenceState {
            newest_request_id: "r2".into(),
            last_applied_version: Some(5),
            newest_ordinal: Some(40),
            newest_deadline_ms: Some(2000),
        };
        let late: StepAnswer = serde_json::from_value(serde_json::json!({
            "answer": "compaction_message",
            "request_id": "r2",
            "compaction": {"compaction_id": "c-late", "version": 6,
                           "range": {"from": 0, "to": 30}, "replacement": []}
        }))
        .unwrap();
        assert_eq!(
            dispose(&timed_out, &late, 2600),
            Disposition::NotApplied(NotApplied::Late)
        );
        assert_eq!(dispose(&timed_out, &late, 1999), Disposition::Act);
    }
}
