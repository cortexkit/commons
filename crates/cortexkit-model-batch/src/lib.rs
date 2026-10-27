//! The batch rules every CortexKit model-batch role shares.
//!
//! A model-batch role (`classifier/v1`, `extractor/v1`) takes a batch of
//! items, calls one hosted model per item, and returns the provider's
//! answers untouched. The roles differ in what they ask the model; they
//! share how a batch is admitted, identified, priced, failed and replayed.
//! `RULES.md`, next to this crate's `Cargo.toml`, states each shared rule
//! once, and each role contract points to it. The code here is those rules:
//!
//! - [`errors`]: the one error vocabulary, its classes, and the item error
//!   and admission refusal shapes;
//! - [`status`]: provider status mapping, with the Cloudflare Workers AI
//!   code table;
//! - [`identity`]: batch identity, hashed as canonical JSON with images in
//!   canonical form;
//! - [`image`]: the item image shape (inline bytes or a reference to an
//!   image another module holds), its decoding validation, the checks a
//!   host runs on a resolved reference, and its canonical form;
//! - [`admission`]: the `batch_id` grammar, bounds, request size, the
//!   catalog row, and the spend ceiling;
//! - [`estimate`]: the conservative cost estimate and nanodollar arithmetic;
//! - [`absent`]: absent is not zero, for usage and cost;
//! - [`number`]: provider number text kept as written;
//! - `vectors` (with the `test-vectors` feature): this crate's vector files.

#![forbid(unsafe_code)]

pub mod absent;
pub mod admission;
pub mod errors;
pub mod estimate;
pub mod identity;
pub mod image;
pub mod number;
pub mod status;
#[cfg(any(test, feature = "test-vectors"))]
pub mod vectors;

/// The major of the shared rules this crate states; each role's `v1` uses
/// these rules.
pub const RULES_VERSION: &str = "v1";
