//! Late results: the names the role pins, and nothing more.
//!
//! A provider that settles calls after its reply declares the `late_results`
//! session capability and serves [`crate::ops::LATE_RESULTS`] `{since}`. The
//! custodian of a result (the scope's owner for a direct call, or the carrier
//! of an onward call) pulls it; the provider never pushes. Only what the role
//! document settles is pinned here: the op, the refusal code, the log's key
//! fields, the cursor's parts and the expired-entry fields. The reply
//! envelope, the entry payload, the ack op and the value types are open
//! items in `CONTRACT.md`, so this module deliberately has no decoder yet.

/// The session capability a provider declares when it may settle calls later.
pub const CAPABILITY: &str = "late_results";

/// A cursor from another provider incarnation, or one past a gap, is refused
/// with this code; the reader re-reads from the start and deduplicates on its
/// own intake key.
pub const CURSOR_INCARNATION_CHANGED: &str = "cursor_incarnation_changed";

/// The fields the durable late-result log is keyed on. `call_key` is
/// qualified by the principal that carried the call; `invocation_id` is
/// present only for an onward request.
pub const LOG_KEY_FIELDS: &[&str] = &[
    "owner",
    "ref",
    "scope_epoch",
    "custodian",
    "call_key",
    "invocation_id",
    "event_id",
];

/// The parts of a `late_results` cursor. The cursor carries the provider's
/// incarnation so a restart that loses the sequence is detected, never read
/// as "nothing new".
pub const CURSOR_FIELDS: &[&str] = &["provider_incarnation", "seq"];

/// What stays of a result that expired unacked, until it is acked.
pub const EXPIRED_ENTRY_FIELDS: &[&str] = &["call_key", "event_id", "settled_at"];
