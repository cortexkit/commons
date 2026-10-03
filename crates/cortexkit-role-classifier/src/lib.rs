//! Wire contract of the `classifier/v1` role.
//!
//! A classifier module takes a batch of items and a set of questions, calls
//! one hosted classifier model for each item, and returns the provider's
//! answers untouched. It claims the role by listing [`PROVIDES`] in its
//! manifest's `capabilities.provides` and answers every op in
//! [`REQUIRED_OPS`].
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document:
//! it marks every rule pinned, and says where each provider shape comes
//! from. The types here are the wire shapes it describes:
//!
//! - [`describe`]: the `role.describe` answer;
//! - [`question`]: the providers' shared question shape, used both in a
//!   `classify.run` request and in the provider request it is forwarded in;
//! - [`request`]: the `classify.run` request and its validators;
//! - [`reply`]: the `classify.run` reply and the provider's answer objects;
//! - [`provider`]: the provider-side request and response;
//! - [`identity`]: the batch body identity that decides replay versus
//!   `batch_id_reuse`;
//! - [`errors`]: admission refusal codes, per-item error codes and classes.
//!
//! Maps whose key order is part of what a provider documents (questions,
//! choice options, probabilities) are [`IndexMap`]s, so a decoded and
//! re-encoded object keeps its order. Every enumeration a consumer decodes
//! (question type, error class) decodes open: an unknown value is kept as a
//! string rather than refused.

#![forbid(unsafe_code)]

pub mod describe;
pub mod errors;
pub mod identity;
pub mod provider;
pub mod question;
pub mod reply;
pub mod request;

pub use indexmap::IndexMap;

/// The role name.
pub const ROLE: &str = "classifier";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The capability identifier a classifier module lists in its manifest's
/// `capabilities.provides`, and the `role` member of its `role.describe`
/// answer.
pub const PROVIDES: &str = "classifier/v1";

/// Names of the ops a classifier module serves.
pub mod ops {
    /// The module's limits and every model it serves, resolved from its
    /// catalog and routing pins. Required.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// Classify a batch of items against a set of questions. Required.
    pub const CLASSIFY_RUN: &str = "classify.run";
}

/// Ops every `classifier/v1` module serves.
pub const REQUIRED_OPS: &[&str] = &[ops::ROLE_DESCRIBE, ops::CLASSIFY_RUN];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provides_is_role_slash_version() {
        assert_eq!(PROVIDES, format!("{ROLE}/{VERSION}"));
    }

    #[test]
    fn provides_is_a_valid_capability_identifier() {
        assert!(subc_protocol::manifest::is_valid_capability_identifier(
            PROVIDES
        ));
    }

    #[test]
    fn required_ops_are_the_role_ops() {
        assert_eq!(REQUIRED_OPS, &["role.describe", "classify.run"]);
    }
}
