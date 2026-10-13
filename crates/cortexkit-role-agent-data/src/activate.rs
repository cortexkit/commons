//! `agent_data.activate {refs, cutId}`: on the receiver, make the staged
//! data live. Refused `external_not_present` until the host program's own
//! data on this Mac matches the `check` the source's `prepare` gave for it
//! (`prepare::ExternalItem::check`); the participant stays at `imported`
//! until then. The refs must be the same set the `import` for this `cutId`
//! staged, in any order; another set is refused `invalid_state` naming the
//! staged refs ([`crate::step::check_ref_set`]).
//! Step: `imported` → `activated`.

/// The `activate` request: `{refs, cutId}`.
pub type ActivateRequest = crate::request::CutRequest;

/// The `activate` reply: `{}`.
pub type ActivateReply = crate::EmptyReply;
