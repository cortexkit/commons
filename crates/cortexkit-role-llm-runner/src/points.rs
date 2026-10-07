//! The kill points the `llm-runner/v1` crash suite will ask a harness for.
//!
//! Each point is a durable state a runner reaches while running a session:
//! a record synced before the step it permits. A kill "at" a point leaves
//! that record on the state root and nothing after it (see
//! `cortexkit-role-harness`). The names are opaque strings to the shared
//! harness; this list is the role's vocabulary, and it names states, not any
//! runner's record types.

/// The session's start record is durable: the frozen plan, composition and
/// role versions. No model request has been sent.
pub const ADMITTED: &str = "Admitted";

/// A send's message is durable with its `send_id` (and, for a steered or
/// queued prompt, its PreUser output). The reply has not been sent. A retry
/// of the same send after a restart answers the existing state and writes
/// nothing new.
pub const SEND_RECORDED: &str = "SendRecorded";

/// A compaction answer to be applied is durable. The request that would
/// carry it has not been sent. Resume applies it without calling the
/// provider again.
pub const COMPACTION_APPLIED: &str = "CompactionApplied";

/// An assistant step is durable with its PostAssistant output. No tool call
/// from it has a dispatch intent yet. The runner either resumes and dispatches
/// each call exactly once, or seals the run `interrupted` without dispatch.
/// The calls are never indeterminate, and no later request carries them
/// without results.
pub const STEP_RECORDED: &str = "StepRecorded";

/// A tool call's dispatch intent is durable, holding the input that
/// executes. The call may or may not have been sent. Resume closes it with
/// an `outcome_unknown` result, never re-dispatches it, and seals the run
/// `interrupted`.
pub const DISPATCH_INTENT: &str = "DispatchIntent";

/// A tool call's result is durable with its PostTool output. Resume replays
/// it without calling the tool or the hook again.
pub const TOOL_RESULT_RECORDED: &str = "ToolResultRecorded";

/// A prefix rebuild that applies a pending change is durable. The request
/// that carries the new prefix has not been sent.
pub const FOLD_RECORDED: &str = "FoldRecorded";

/// The run's terminal state is durable. Resume never writes a second one.
pub const TERMINAL: &str = "Terminal";

/// Expiry's content-free tombstone is durable, but transcript and derived
/// copies have not yet been deleted. Restart must finish the deletion.
pub const RETENTION_TOMBSTONED: &str = "RetentionTombstoned";

/// Every point in the role's vocabulary, in the order a run that uses them
/// all reaches them.
pub const ALL: &[&str] = &[
    ADMITTED,
    SEND_RECORDED,
    COMPACTION_APPLIED,
    FOLD_RECORDED,
    STEP_RECORDED,
    DISPATCH_INTENT,
    TOOL_RESULT_RECORDED,
    TERMINAL,
    RETENTION_TOMBSTONED,
];

#[cfg(test)]
mod tests {
    #[test]
    fn point_names_are_distinct() {
        let mut names = super::ALL.to_vec();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), super::ALL.len());
    }
}
