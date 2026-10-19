//! The kill points the `compaction-provider/v1` crash suite will ask a
//! harness for.
//!
//! Each point is a durable state the PROVIDER reaches while serving a
//! session: a record in its own store, synced before the step it permits.
//! A kill "at" a point leaves that record on the state root and nothing
//! after it (see `cortexkit-role-harness`). The suite drives the provider
//! with a scripted runner, so the runner's own points are not in this list.
//! The names are opaque strings to the shared harness; this list is the
//! role's vocabulary.

/// The provider's per-session Setup state is durable; the Setup answer has
/// not been sent. A Setup repeated after the restart gets a well-formed
/// answer for the same session.
pub const SETUP_RECORDED: &str = "SetupRecorded";

/// Messages the provider read with `session.read` are durable in its store;
/// its cursor has not moved past them. After the restart it reads them again
/// from its cursor and takes in each ordinal once.
pub const MESSAGES_INGESTED: &str = "MessagesIngested";

/// A view is durable in the provider's store under its version; its
/// `view_ready` event has not been published. After the restart the provider
/// publishes that view's event, or a newer view at a higher version; it never
/// publishes different content under the recorded version.
pub const VIEW_RECORDED: &str = "ViewRecorded";

/// Every point in the role's vocabulary, in the order a session that uses
/// them all reaches them.
pub const ALL: &[&str] = &[SETUP_RECORDED, MESSAGES_INGESTED, VIEW_RECORDED];

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
