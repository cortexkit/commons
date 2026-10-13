//! `agent_data.abort {refs, cutId}`: on either side, only before the move
//! record reads `activated`. The source unfreezes and removes its durable
//! freeze; the receiver drops its staging. Aborting a `cutId` this
//! participant never saw succeeds and touches no data. Refused
//! `invalid_state` after `activated` (receiver) or `released` (source), or
//! when the refs are not the set the `cutId` started with (`prepare`'s on
//! the source, `import`'s on the receiver; [`crate::step::check_ref_set`]).
//! Step: any earlier step → `aborted`.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

/// The `abort` request: `{refs, cutId}`.
pub type AbortRequest = crate::request::CutRequest;

/// The `abort` reply: `{}`.
pub type AbortReply = crate::EmptyReply;
