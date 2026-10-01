//! The kill points the `step-transform-provider/v1` crash suite will ask a
//! harness for.
//!
//! A step-transform provider holds little durable state of its own: the
//! runner writes every hook output on the record it transforms, and replays
//! it without calling the provider again. What a provider may keep is
//! per-subject state behind an answer (a tag number it assigned, say). The
//! runner re-runs the hooks of a step whose outputs never became durable,
//! so the provider must answer a repeated call for the same subject as a
//! fresh one, and nothing it kept for an answer that was never used may
//! reach the model.
//!
//! The suite drives the provider with a scripted runner; the runner's own
//! points (`llm-runner/v1`'s `StepRecorded`, `DispatchIntent`,
//! `ToolResultRecorded`, `SendRecorded`) are not in this list. The names are
//! opaque strings to the shared harness; this list is the role's
//! vocabulary.

/// The provider has durably recorded state behind a hook answer it has not
/// sent. After the restart, the same call gets a well-formed answer, a
/// `mutate` answer is byte-identical to the one the provider would have
/// sent, and state kept for the unsent answer never surfaces in a later
/// answer.
pub const HOOK_STATE_RECORDED: &str = "HookStateRecorded";

/// Every point in the role's vocabulary.
pub const ALL: &[&str] = &[HOOK_STATE_RECORDED];

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
