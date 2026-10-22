//! Wire contract of the `extractor/v1` role: the request side.
//!
//! An extractor module takes a batch of items and a set of typed fields,
//! calls one hosted extraction model for each item with every field, and
//! returns the provider's answers untouched. It claims the role by listing
//! [`PROVIDES`] in its manifest's `capabilities.provides` and answers every
//! op in [`REQUIRED_OPS`].
//!
//! `CONTRACT.md`, next to this crate's `Cargo.toml`, is the role document.
//! The batch rules this role shares with `classifier/v1` are
//! `cortexkit-model-batch`'s `RULES.md`, re-exported here where this role
//! uses them. The modules:
//!
//! - [`describe`]: the `role.describe` answer;
//! - [`field`]: the nine field types, their options, and the field check;
//! - [`request`]: the `extract.run` request, its validators, the reply-size
//!   check, and [`request::admit`], every admission step in order;
//! - [`identity`]: the batch body identity;
//! - [`spend`]: the pre-call spend check with the shared estimate;
//! - [`provider`]: the Clairic request mapping and reply envelope;
//! - [`reply`]: the reply and the provider's answer objects, as far as they
//!   are measured;
//! - [`errors`]: the codes this role answers with;
//! - [`image`]: the shared item image shape;
//! - `vectors` (with the `test-vectors` feature): the role's vector files.

#![forbid(unsafe_code)]

pub mod describe;
pub mod errors;
pub mod field;
pub mod identity;
pub mod image;
pub mod provider;
pub mod reply;
pub mod request;
pub mod spend;
#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;

pub use indexmap::IndexMap;

/// The role name.
pub const ROLE: &str = "extractor";

/// The only major this crate defines.
pub const VERSION: &str = "v1";

/// The capability identifier an extractor module lists in its manifest's
/// `capabilities.provides`, and the `role` member of its `role.describe`
/// answer.
pub const PROVIDES: &str = "extractor/v1";

/// Names of the ops an extractor module serves.
pub mod ops {
    /// The module's limits and every model it serves. Required.
    pub const ROLE_DESCRIBE: &str = "role.describe";
    /// Extract every field from each item of a batch. Required.
    pub const EXTRACT_RUN: &str = "extract.run";
}

/// Ops every `extractor/v1` module serves.
pub const REQUIRED_OPS: &[&str] = &[ops::ROLE_DESCRIBE, ops::EXTRACT_RUN];

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
        assert_eq!(REQUIRED_OPS, &["role.describe", "extract.run"]);
    }
}
