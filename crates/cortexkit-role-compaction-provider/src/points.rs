//! The kill points the `compaction-provider/v1` crash suite will ask a
//! harness for.
//!
//! Each point is a durable state the PROVIDER reaches while serving a
//! session: a record in its own store, synced before the step it permits.
//! A kill "at" a point leaves that record on the state root and nothing
//! after it (see `cortexkit-role-harness`). The suite drives the provider
//! with a scripted runner, so the runner's own points (`llm-runner/v1`'s
//! `CompactionApplied`, say) are not in this list. The names are opaque
//! strings to the shared harness; this list is the role's vocabulary.

/// The provider's per-session Setup state is durable; the Setup answer has
/// not been sent. A Setup repeated after the restart gets a well-formed
/// answer for the same session.
pub const SETUP_RECORDED: &str = "SetupRecorded";

/// The messages a status carried are durable in the provider's store; no
/// answer has been sent, so the runner's cursor has not moved. The same
/// messages arrive again in the next status, and the provider takes in each
/// ordinal once.
pub const MESSAGES_INGESTED: &str = "MessagesIngested";

/// The provider has durably allocated the version of the CompactionMessage
/// it is about to send; the answer has not been sent. Its next
/// CompactionMessage carries a higher version, and no version is used for
/// two different contents.
pub const ANSWER_RECORDED: &str = "AnswerRecorded";

/// The provider answered `wait`; the work behind it is not durable yet. The
/// next status gets a normal answer, and no CompactionMessage is built from
/// the lost work.
pub const WAIT_ANSWERED: &str = "WaitAnswered";

/// The work behind a `wait` is durable; `compaction.ready` has not been
/// sent. After the restart the provider sends `compaction.ready` for the
/// newest `wait` it answered, or answers the next status with the result;
/// the work is not lost.
pub const WAIT_WORK_DURABLE: &str = "WaitWorkDurable";

/// Every point in the role's vocabulary, in the order a session that uses
/// them all reaches them.
pub const ALL: &[&str] = &[
    SETUP_RECORDED,
    MESSAGES_INGESTED,
    ANSWER_RECORDED,
    WAIT_ANSWERED,
    WAIT_WORK_DURABLE,
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
