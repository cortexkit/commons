//! Scripted checks for media-provider/v1 adapters.
//!
//! Each case needs a fresh subject. Hooks default to explicit, named errors so
//! an adapter cannot accidentally pass a case without installing its fixtures.

pub mod checks;
mod describe_checks;
pub mod fixtures;
pub mod peers;
pub mod rule_map;
pub mod subject;

pub use checks::{run_case, Outcome, CASES};
pub use subject::MediaSubject;

#[cfg(test)]
mod tests;
