//! `agent_data.activate {refs, cutId}`: on the receiver, make the staged
//! data live. Refused `external_not_present` until the host program's own
//! data on this Mac matches the `check` the source's `prepare` gave for it
//! (`prepare::ExternalItem::check`), and refused `busy` while the
//! participant's own data for these refs is in use and can't go live yet
//! (the driver retries later). Either way the participant stays at
//! `imported` until it succeeds. The participant re-checks divergence
//! atomically with swapping the staged data in; if this Mac's copy gained
//! data after `diverged` answered `at_cut`, it answers `local_advanced`
//! instead of overwriting, leaving the staged import and its local copy
//! untouched. That is terminal for the step, and unlike after `import` the
//! driver cannot abort: the receiver already holds the claim, other
//! participants may already be live, and `abort` after `activate` is
//! refused. The move holds as a fork until the operator resolves it. The refs must be the same set the `import` for this `cutId`
//! staged, in any order; another set is refused `invalid_state` naming the
//! staged refs ([`crate::step::check_ref_set`]).
//! Step: `imported` → `activated`.
//!
//! The contract terms used here (source, receiver, cut, the door, driver
//! and the rest) are defined in the "Terms" section of `CONTRACT.md`.

/// The `activate` request: `{refs, cutId}`.
pub type ActivateRequest = crate::request::CutRequest;

/// The `activate` reply: `{}`.
pub type ActivateReply = crate::EmptyReply;
