//! Wire contract for `compaction-provider/v2` and `compaction-runner/v1`.
//!
//! A runner holds a session's transcript and renders every request to the
//! model. A compaction provider decides how that history is shortened, and
//! says so as a view: a list of edits the runner applies to its transcript
//! before every request. The runner knows nothing about how history should
//! be edited; it applies a view exactly as written, or refuses it by name.
//!
//! Each side calls the other on a route bound to one session: the route's
//! bind identity names the session, the daemon attests the caller, and the
//! daemon's scope stamp names the session's folder and project. No request
//! body names a runner, a session or a project.
//!
//! What flows between them, all as calls over their routes:
//! - the runner presents its transcript in one neutral part model
//!   ([`part`], [`read`]), whatever its host stores, so a provider needs no
//!   decoder per harness;
//! - the runner calls Setup when it starts serving a session, and again
//!   whenever the provider has lost it ([`setup`]); on the W5.2 notice
//!   triggers it sends new and changed entries in compact form
//!   ([`notice`]); and before a request that rewrites the model provider's
//!   prompt cache anyway, or that would not fit, it asks for a view and waits
//!   for the answer ([`ride`]);
//! - the provider hands over new views with `session.apply_view` ([`view`]),
//!   or in a ride's answer; the runner
//!   checks each one against the rules it declared ([`rules`]), stores it, and
//!   applies it from the next step until a newer one arrives.
//!
//! This crate holds only what goes over the wire, plus the checksums that are
//! part of it ([`digest`]). How a view is rendered and checked lives in
//! a separate view library, which depends on this crate:
//! runners render with it, and providers use the same functions to know
//! exactly what the model sees.

#![forbid(unsafe_code)]

pub use cortexkit_role_llm_runner::rides;

#[macro_use]
mod open;

mod integer;

pub mod digest;
pub mod notice;
pub mod part;
pub mod read;
pub mod ride;
pub mod rules;
pub mod setup;
pub mod signed_floor;
mod statement_order;
pub mod tokens;
pub mod view;
pub mod wire;

pub use signed_floor::SignedFloor;

/// The capability a compaction provider lists in its manifest. The major is
/// new because every op below changed shape incompatibly.
pub const COMPACTION_PROVIDES: &str = "compaction-provider/v2";

/// Additional provider manifest capability enabling `Entry.speaks_for`.
/// Runners read the provider's catalog entry at every Setup and retain that
/// declaration until the next Setup. Providers must not withdraw it while
/// serving a session without restarting.
pub const COMPACTION_ENTRY_SPEAKS_FOR_PROVIDES: &str = "compaction-entry-speaks-for/v1";

/// Additional provider manifest capability enabling Setup and Conditions
/// `signed_floor`. Runners send the boundary only to declaring providers.
pub const COMPACTION_SIGNED_FLOOR_PROVIDES: &str = "compaction-signed-floor/v1";

/// Additional provider manifest capability enabling Setup and Conditions
/// `statement_epoch` and `statement_seq`. Runners send both only to declaring
/// providers on contract version 0.3.8 or later.
pub const COMPACTION_STATEMENT_SEQ_PROVIDES: &str = "compaction-statement-seq/v1";

/// Additional provider manifest capability enabling Conditions `notes_tokens`.
/// Runners separate notes from `system_tokens` only for declaring providers.
pub const COMPACTION_NOTES_TOKENS_PROVIDES: &str = "compaction-notes-tokens/v1";

/// Additional provider manifest capability enabling named-read caps and
/// continuation, Conditions `window_over_budget`, and the matching ride cause.
/// Deploy readers first.
/// After-read page ceilings and first-notice deltas need no declaration.
pub const COMPACTION_LIMITS_PROVIDES: &str = "compaction-limits/v1";

/// Whether a provider declares support for runner-owned named-read and window bounds.
pub fn provides_limits(provides: &[String]) -> bool {
    provides
        .iter()
        .any(|capability| capability == COMPACTION_LIMITS_PROVIDES)
}

/// Whether a provider declares support for the next request's note count.
pub fn provides_notes_tokens(provides: &[String]) -> bool {
    provides
        .iter()
        .any(|capability| capability == COMPACTION_NOTES_TOKENS_PROVIDES)
}

/// Whether a provider declares support for the runner's signed floor.
pub fn provides_signed_floor(provides: &[String]) -> bool {
    provides
        .iter()
        .any(|capability| capability == COMPACTION_SIGNED_FLOOR_PROVIDES)
}

/// Whether a provider declares support for runner-state statement ordering.
pub fn provides_statement_seq(provides: &[String]) -> bool {
    provides
        .iter()
        .any(|capability| capability == COMPACTION_STATEMENT_SEQ_PROVIDES)
}

/// Whether a provider's catalog capabilities declare agent attribution.
/// Evaluate at Setup, not from the runner's own Setup request fields.
pub fn provides_entry_speaks_for(provides: &[String]) -> bool {
    provides
        .iter()
        .any(|capability| capability == COMPACTION_ENTRY_SPEAKS_FOR_PROVIDES)
}

/// The role and version a runner declares when it opens a route for this
/// contract (`role_versions` on the route), so the provider knows which
/// contract the route speaks. Routes are per session and per role: a runner
/// opens one route for compaction, one for tools and one for hooks to the
/// same provider, each bound to the session's identity and scope.
pub const COMPACTION_ROLE: (&str, &str) = ("compaction-provider", "v2");

/// The capability a runner declares for its side of this contract:
/// `session.entries`, `session.apply_view` and `session.applied_view`. It
/// pairs with [`COMPACTION_PROVIDES`] by name; the name is new, so it starts
/// at v1.
pub const COMPACTION_RUNNER_PROVIDES: &str = "compaction-runner/v1";

/// The role a provider declares on the route it opens to the runner for this
/// contract's runner ops.
pub const COMPACTION_RUNNER_ROLE: (&str, &str) = ("compaction-runner", "v1");

/// The role a provider declares on the route it opens to the runner for the
/// llm-runner ops it uses as published (`session.send`, `run.result`). This
/// contract does not change llm-runner.
pub const LLM_RUNNER_ROLE: (&str, &str) = ("llm-runner", "v1");

/// Names of the ops, used as the `method` of the request that carries them.
/// The prefix names the side that serves the op: `compaction.*` the
/// compaction provider, `session.*` and `run.*` the runner.
pub mod ops {
    /// Served by the provider: start managing a session ([`crate::setup`]).
    pub const COMPACTION_SETUP: &str = "compaction.setup";
    /// Served by the provider: transcript changes and other W5.2 notice
    /// triggers ([`crate::notice`]), not count metadata updates alone.
    pub const COMPACTION_NOTICE: &str = "compaction.notice";
    /// Served by the provider: the runner asks for a view now ([`crate::ride`]).
    pub const COMPACTION_RIDE: &str = "compaction.ride";
    /// Served by the runner (compaction-runner): read transcript entries in
    /// the neutral part model ([`crate::read`]). A name of its own, so logs
    /// never show two shapes of llm-runner's `session.read`.
    pub const SESSION_ENTRIES: &str = "session.entries";
    /// Served by the runner (compaction-runner): take a new view
    /// ([`crate::view::ApplyViewRequest`]).
    pub const SESSION_APPLY_VIEW: &str = "session.apply_view";
    /// Served by the runner (compaction-runner): the view it applies now
    /// ([`crate::view::AppliedViewRequest`]).
    pub const SESSION_APPLIED_VIEW: &str = "session.applied_view";
    /// Served by the runner (llm-runner): queue text for the model, or start
    /// a run in a child session (see [`cortexkit_role_llm_runner::send`]).
    pub const SESSION_SEND: &str = "session.send";
    /// Served by the runner (llm-runner): a run's state and reply
    /// (see [`cortexkit_role_llm_runner::run`]).
    pub const RUN_RESULT: &str = "run.result";
}
