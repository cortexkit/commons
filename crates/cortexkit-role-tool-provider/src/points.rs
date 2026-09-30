//! The kill points the `tool-provider/v1` crash suite asks a harness for.
//!
//! These are the durable states an approval-gated call moves through on the
//! provider that executes it. Each is a record synced before the step it
//! permits, so a kill "at" a point leaves that record on disk and nothing
//! after it. The names are opaque strings to the shared harness; this list is
//! the role's vocabulary.

/// The call is held and its question filed; nothing is authorized or sent.
/// A provider restarted here reports the call as not started and never runs
/// it.
pub const PREPARED: &str = "Prepared";

/// The approval decision is recorded, and nothing has been sent. A kill here
/// is the cut "after Authorized, before DispatchStarted". A provider
/// restarted here reports the call as not started and never runs it.
pub const AUTHORIZED: &str = "Authorized";

/// Sending may have begun. A provider restarted here reports what it observes
/// of the call, never a guessed outcome. Declared so harnesses can name it;
/// the v1 suite does not cut here yet.
pub const DISPATCH_STARTED: &str = "DispatchStarted";

/// Every point in the role's vocabulary, in the order a call reaches them.
pub const ALL: &[&str] = &[PREPARED, AUTHORIZED, DISPATCH_STARTED];
