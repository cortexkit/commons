//! The view library of the compaction contract: everything a runner and a
//! compaction provider must compute the same way.
//!
//! A runner renders every request from a view, and the provider decides
//! what goes into the view from what that rendering will be. If the two
//! computed it separately, they would drift, and a provider would plan
//! against a request the model never receives. So both call these functions:
//! - [`render::plan`]: what a runner renders, as keep, drop and replace
//!   instructions over its own host messages plus the inserted messages. It
//!   validates edit structure and targets, not the view checksum, version,
//!   `since` provenance, or provider-rule compliance; check those separately;
//! - [`render::apply`] and [`render::apply_edits`]: the same result in the
//!   neutral part model, which is what the provider plans against;
//! - [`render::check_carried`]: that each edit of a new view claims an honest
//!   `since`, so no new edit is hidden from the rule check by backdating it.
//!   It does not check that the view carries every earlier edit (V2): an
//!   edit also leaves when a later cut covers its message or a revert removes
//!   it, and only the provider holds those rules, so carrying edits forward
//!   is the provider's obligation;
//! - [`rules::check`]: whether a view keeps every signed part valid under the
//!   rules the runner declared, and [`rules::defaults`] for a model;
//! - [`tags`]: which parts take a number, how a runner counts them and how it
//!   writes them;
//! - [`clean::clean_reply`]: a cleaned candidate for an assistant reply. The
//!   caller applies the contract's empty-text policy when mapping and rendering
//!   replies; whether stored host text is also cleaned is the runner's choice.
//!
//! The wire types live in the contract crate, `cortexkit-role-compaction`;
//! this library depends on it. A runner written in
//! another language uses the WebAssembly build of this library, or checks
//! its own implementation against the JSON test vectors under `vectors/`.

#![forbid(unsafe_code)]

/// Rendering-semantics version. Bump it whenever supported rendering behavior
/// changes. A regression test pins representative JSON outputs for selected
/// rendering operations and a sample tag prefix. It is not exhaustive and does
/// not hash final host requests; retaining historical pins and requiring version
/// bumps are release-review obligations. Runners requiring replay compatibility
/// must persist/check the version themselves.
pub const PLAN_VERSION: u32 = 2;

pub mod clean;
pub mod render;
pub mod rules;
pub mod tags;
pub mod vectors;

#[cfg(test)]
mod tests;
